#!/usr/bin/env node
/**
 * Command-line assembly only.
 *
 * Every command parses options, calls one session or document method and prints
 * the result. The session owns native access and the file-backed history, so
 * this file holds no persistence, no drawing logic and no `service` calls.
 *
 * Commands that edit an existing file open its document, mutate it and let the
 * document write the file back; `history: 'sidecar'` keeps undo/redo working
 * across separate CLI processes.
 */

import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

import { createCli, type CommandContext } from '@wae/commander';

import { OpenCADError } from './errors.js';
import { runGui } from './gui.js';
import { OpenCAD } from './open-cad.js';
import type { DrawingDocument } from './document.js';
import type {
    CopyResult,
    Drawing,
    Entity,
    EntityChange,
    EntityDeletion,
    ExportFormat,
    HistoryResult,
    ImportLoss,
    InspectCheck,
    InspectReport,
    LayerChange,
    LayerSummary,
    MeasureResult,
    OpenCADCapabilities,
    Point,
    SaveReport,
    TransformOperation,
    TransformResult,
    TransformTarget,
} from './types.js';

const here = path.dirname(fileURLToPath(import.meta.url));

const cli = createCli('open-cad').locales(path.join(here, '..', 'locales')).intro('cli.intro');

const app = new OpenCAD({ history: 'sidecar' });

// Exported for contract audits that import the built entry to inspect the
// command tree. Nothing runs until the file is executed as the program entry.
export { cli };

interface ParsedOptions {
    readonly _?: readonly string[];
    readonly [key: string]: unknown;
}

const text = (options: ParsedOptions, key: string): string | undefined =>
    typeof options[key] === 'string' ? (options[key] as string) : undefined;

const flag = (options: ParsedOptions, key: string): boolean => options[key] === true || options[key] === 'true';

const input = (options: ParsedOptions): string | undefined => options._?.[0];

const number = (options: ParsedOptions, key: string): number | undefined => {
    const raw = text(options, key);
    if (raw === undefined || raw.trim().length === 0) {
        return undefined;
    }
    const value = Number(raw);
    return Number.isFinite(value) ? value : undefined;
};

const toggle = (options: ParsedOptions, key: string): boolean | undefined => {
    const raw = text(options, key);
    if (raw === 'on') {
        return true;
    }
    return raw === 'off' ? false : undefined;
};

/**
 * Runs one session or document call and routes a domain failure through
 * `ctx.fail`, so the stable code (for example `drawing/layer-locked`) reaches
 * the CLI envelope instead of being folded into `commander::action_failed`.
 */
const guard = async <T>(ctx: CommandContext, work: () => T | Promise<T>): Promise<T> => {
    try {
        return await work();
    }
    catch (error) {
        if (error instanceof OpenCADError) {
            ctx.fail(error.code, { message: error.message });
        }
        throw error;
    }
};

/**
 * Document edits return the created/updated object, so the CLI rebuilds the
 * change envelope from the document's committed summary for its JSON output.
 */
const entityChange = (document: DrawingDocument, entity: Entity): EntityChange => ({
    revision: document.revision,
    drawing: document.summary(),
    entity,
});

const layerChange = (document: DrawingDocument, layer: LayerSummary): LayerChange => ({
    revision: document.revision,
    drawing: document.summary(),
    layer,
});

const point = (options: ParsedOptions, key: string): Point | undefined => {
    const raw = text(options, key);
    if (raw === undefined) {
        return undefined;
    }
    const parts = raw.split(',');
    if (parts.length !== 2) {
        return undefined;
    }
    const x = Number(parts[0].trim());
    const y = Number(parts[1].trim());
    return Number.isFinite(x) && Number.isFinite(y) ? [x, y] : undefined;
};

const idList = (options: ParsedOptions, key: string): string[] | undefined => {
    const raw = text(options, key);
    if (raw === undefined) {
        return undefined;
    }
    const ids = raw
        .split(',')
        .map((entry) => entry.trim())
        .filter((entry) => entry.length > 0);
    return ids.length > 0 ? ids : undefined;
};

const points = (options: ParsedOptions, key: string): Point[] | undefined => {
    const raw = text(options, key);
    if (raw === undefined) {
        return undefined;
    }
    const parsed: Point[] = [];
    for (const entry of raw.split(/[\s;]+/).filter((part) => part.length > 0)) {
        const parts = entry.split(',');
        if (parts.length !== 2) {
            return undefined;
        }
        const x = Number(parts[0].trim());
        const y = Number(parts[1].trim());
        if (!Number.isFinite(x) || !Number.isFinite(y)) {
            return undefined;
        }
        parsed.push([x, y]);
    }
    return parsed.length > 0 ? parsed : undefined;
};

function describeCapabilities(matrix: OpenCADCapabilities): string {
    const level = (entries: [string, string][]): string =>
        entries.map(([name, support]) => (support === 'supported' ? name : `${name} (${support})`)).join(', ');
    const formats = level(Object.entries(matrix.formats).filter(([, support]) => support !== 'unsupported'));
    const checks = Object.entries(matrix.checks)
        .filter(([, support]) => support === 'supported')
        .map(([name]) => name)
        .join(', ');
    const exports = level(Object.entries(matrix.exportTargets).filter(([, support]) => support !== 'unsupported'));
    return [
        `engine: ${matrix.engine.package} ${matrix.engine.version}`,
        `drawing formats: ${formats.length > 0 ? formats : 'none supported'}`,
        `checks: ${checks.length > 0 ? checks : 'none supported'}`,
        `export targets: ${exports.length > 0 ? exports : 'none supported'}`,
        `project: ${matrix.project}`,
    ].join('\n');
}

function describeDrawing(drawing: Drawing): string {
    const layers = drawing.layers.map((layer) => `${layer.name}(${layer.entityCount})`);
    return [
        `drawing ${drawing.id} revision=${drawing.revision} entities=${drawing.entities} units=${drawing.units}`,
        `  title: ${drawing.title ?? '(untitled)'}`,
        `  layers: ${layers.length > 0 ? layers.join(', ') : 'none'}`,
    ].join('\n');
}

function describeReport(report: InspectReport): string {
    const lines = [`inspect revision=${report.revision} checker=${report.checkerVersion} passed=${report.passed}`];
    for (const issue of report.issues) {
        lines.push(`  ${issue.severity} ${issue.code} ${issue.layerId ?? '-'} ${issue.objectId ?? '-'} ${issue.message}`);
    }
    return lines.join('\n');
}

function describeSave(report: SaveReport): string {
    return `saved ${report.path} revision=${report.revision} bytes=${report.bytes}`;
}

function describeEntity(change: EntityChange): string {
    return `added ${change.entity.id} (${change.entity.kind.type}) on layer ${change.entity.layer} revision=${change.revision}`;
}

function describeEdit(change: EntityChange, verb: 'moved' | 'updated'): string {
    return `${verb} ${change.entity.id} (${change.entity.kind.type}) layer=${change.entity.layer} revision=${change.revision}`;
}

function describeDeletion(deletion: EntityDeletion): string {
    return `deleted ${deletion.deleted} revision=${deletion.revision} entities=${deletion.drawing.entities}`;
}

function describeEntities(entities: readonly Entity[]): string {
    if (entities.length === 0) {
        return 'no entities';
    }
    return entities.map((entity) => `${entity.id} ${entity.kind.type} layer=${entity.layer}`).join('\n');
}

function describeTransform(result: TransformResult): string {
    const bounds = result.bounds;
    return `transformed ${result.affected} entities revision=${result.revision} bounds=${bounds.minX},${bounds.minY} ${bounds.maxX},${bounds.maxY}`;
}

function describeCopy(result: CopyResult): string {
    return `copied ${result.count} entities revision=${result.revision} created=${result.created.join(',')}`;
}

function describeMeasure(result: MeasureResult): string {
    const bounds = result.bounds;
    return [
        `measured ${result.entities} entities revision=${result.revision}`,
        `  bounds: ${bounds.minX},${bounds.minY} ${bounds.maxX},${bounds.maxY}`,
        `  size: ${result.width} x ${result.height}`,
        `  length: ${result.length}`,
        `  area: ${result.area}`,
    ].join('\n');
}

function describeHistory(result: HistoryResult, verb: 'undo' | 'redo'): string {
    return `${verb} revision=${result.revision} entities=${result.drawing.entities} canUndo=${result.canUndo} canRedo=${result.canRedo}`;
}

function describeLayer(change: LayerChange): string {
    const layer = change.layer;
    return `layer ${layer.id} name=${layer.name} color=${layer.color} visible=${layer.visible} locked=${layer.locked} entities=${layer.entityCount} revision=${change.revision}`;
}

function describeImport(
    source: string,
    counts: { readonly entities: number; readonly layers: number },
    losses: readonly ImportLoss[],
    save: SaveReport,
): string {
    const lines = [`imported ${counts.entities} entities and ${counts.layers} layers from ${source}`, describeSave(save)];
    for (const loss of losses) {
        lines.push(`  ${loss.severity} ${loss.code} ${loss.detail}`);
    }
    return lines.join('\n');
}

cli.command('capabilities', 'cli.cmd.capabilities').actionCtx(async (_options, _args, ctx) => {
    const matrix = await guard(ctx, () => app.capabilities());
    ctx.emitResult(matrix, describeCapabilities(matrix));
    return 0;
});

cli
    .command('create', 'cli.cmd.create')
    .option('--out <file>', 'cli.opt.out')
    .option('--title <title>', 'cli.opt.title')
    .option('--units <unit>', 'cli.opt.units', { enum: ['mm', 'inch'] })
    .option('--template <name>', 'cli.opt.template')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const out = text(options, 'out');
        if (out === undefined) {
            ctx.fail('open-cad.cli.out-required', { message: '`--out <file>` is required' });
            return 2;
        }
        const units = text(options, 'units') as 'mm' | 'inch' | undefined;
        const created = await guard(ctx, () =>
            app.create({ title: text(options, 'title'), units, template: text(options, 'template') }),
        );
        try {
            const report = await guard(ctx, () => created.save({ target: out, overwrite: flag(options, 'overwrite') }));
            ctx.emitResult({ ...report, drawing: created.drawing }, describeSave(report));
            return 0;
        } finally {
            created.close();
        }
    });

cli.command('open', 'cli.cmd.open').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
        return 2;
    }
    const opened = await guard(ctx, () => app.open({ path: file }));
    try {
        ctx.emitResult(opened.drawing, describeDrawing(opened.drawing));
        return 0;
    } finally {
        opened.close();
    }
});

cli
    .command('inspect', 'cli.cmd.inspect')
    .option('--checks <list>', 'cli.opt.checks')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const checks = text(options, 'checks')
            ?.split(',')
            .map((name) => name.trim())
            .filter((name) => name.length > 0) as InspectCheck[] | undefined;
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const report = await guard(ctx, () => opened.inspect({ checks }));
            ctx.emitResult(report, describeReport(report));
            return report.passed ? 0 : 1;
        } finally {
            opened.close();
        }
    });

cli
    .command('export', 'cli.cmd.export')
    .option('--format <format>', 'cli.opt.format', { enum: ['dxf', 'step', 'stl', 'svg', 'pdf'] })
    .option('--out <file>', 'cli.opt.out')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const format = text(options, 'format') as ExportFormat | undefined;
        if (format === undefined) {
            ctx.fail('open-cad.cli.format-required', { message: '`--format` is required' });
            return 2;
        }
        const out = text(options, 'out');
        if (out === undefined) {
            ctx.fail('open-cad.cli.out-required', { message: '`--out <file>` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const report = await guard(ctx, () => opened.export({ format, target: out }));
            ctx.emitResult(report, `exported ${report.target} format=${report.format} revision=${report.revision}`);
            return 0;
        } finally {
            opened.close();
        }
    });

for (const [name, description] of [
    ['undo', 'cli.cmd.undo'],
    ['redo', 'cli.cmd.redo'],
] as const) {
    cli.command(name, description).actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => (name === 'undo' ? opened.undo() : opened.redo()));
            ctx.emitResult(result, describeHistory(result, name));
            return 0;
        } finally {
            opened.close();
        }
    });
}

cli
    .command('import', 'cli.cmd.import')
    .option('--out <file>', 'cli.opt.out')
    .option('--title <title>', 'cli.opt.title')
    .option('--overwrite', 'cli.opt.overwrite')
    .actionCtx(async (options, _args, ctx) => {
        const source = input(options);
        if (source === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a source DXF file is required' });
            return 2;
        }
        const out = text(options, 'out');
        if (out === undefined) {
            ctx.fail('open-cad.cli.out-required', { message: '`--out <file>` is required' });
            return 2;
        }
        const imported = await guard(ctx, () => app.importDxf({ source, title: text(options, 'title') }));
        try {
            const report = await guard(ctx, () => imported.save({ target: out, overwrite: flag(options, 'overwrite') }));
            const counts = imported.imported ?? { entities: imported.entities, layers: imported.layers.length };
            const losses = imported.losses ?? [];
            ctx.emitResult(
                { ...report, drawing: imported.drawing, imported: counts, losses },
                describeImport(source, counts, losses, report),
            );
            return 0;
        } finally {
            imported.close();
        }
    });

const entity = cli.command('entity', 'cli.cmd.entity');

entity
    .command('add-line', 'cli.cmd.add-line')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--from <x,y>', 'cli.opt.from')
    .option('--to <x,y>', 'cli.opt.to')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const from = point(options, 'from');
        const to = point(options, 'to');
        if (from === undefined || to === undefined) {
            ctx.fail('open-cad.cli.point-required', { message: '`--from x,y` and `--to x,y` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addLine({ layer: text(options, 'layer') ?? '0', from, to }),
            );
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

entity
    .command('add-circle', 'cli.cmd.add-circle')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--center <x,y>', 'cli.opt.center')
    .option('--radius <number>', 'cli.opt.radius')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const center = point(options, 'center');
        const radius = number(options, 'radius');
        if (center === undefined || radius === undefined) {
            ctx.fail('open-cad.cli.circle-required', { message: '`--center x,y` and `--radius <number>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addCircle({ layer: text(options, 'layer') ?? '0', center, radius }),
            );
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

entity
    .command('add-polyline', 'cli.cmd.add-polyline')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--points <list>', 'cli.opt.points')
    .option('--closed', 'cli.opt.closed')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const parsed = points(options, 'points');
        if (parsed === undefined) {
            ctx.fail('open-cad.cli.points-required', { message: '`--points "0,0 10,0 10,10"` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addPolyline({
                    layer: text(options, 'layer') ?? '0',
                    points: parsed,
                    closed: flag(options, 'closed'),
                }),
            );
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

entity
    .command('add-text', 'cli.cmd.add-text')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--at <x,y>', 'cli.opt.at')
    .option('--height <number>', 'cli.opt.height')
    .option('--text <text>', 'cli.opt.text')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const at = point(options, 'at');
        const height = number(options, 'height');
        const content = text(options, 'text');
        if (at === undefined || height === undefined || content === undefined) {
            ctx.fail('open-cad.cli.text-required', { message: '`--at x,y`, `--height <number>` and `--text <text>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addText({ layer: text(options, 'layer') ?? '0', at, height, content }),
            );
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

entity
    .command('add-dimension', 'cli.cmd.add-dimension')
    .option('--layer <layer>', 'cli.opt.layer', { default: 'dimensions' })
    .option('--from <x,y>', 'cli.opt.from')
    .option('--to <x,y>', 'cli.opt.to')
    .option('--offset <number>', 'cli.opt.offset')
    .option('--text <text>', 'cli.opt.text')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const from = point(options, 'from');
        const to = point(options, 'to');
        const offset = number(options, 'offset');
        if (from === undefined || to === undefined || offset === undefined) {
            ctx.fail('open-cad.cli.dimension-required', { message: '`--from x,y`, `--to x,y` and `--offset <number>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addDimension({
                    layer: text(options, 'layer') ?? 'dimensions',
                    from,
                    to,
                    offset,
                    text: text(options, 'text'),
                }),
            );
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

entity
    .command('delete', 'cli.cmd.delete')
    .option('--id <entity>', 'cli.opt.id')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        if (id === undefined) {
            ctx.fail('open-cad.cli.id-required', { message: '`--id <entity>` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const deletion = await guard(ctx, () => opened.deleteEntity(id));
            ctx.emitResult(deletion, describeDeletion(deletion));
            return 0;
        } finally {
            opened.close();
        }
    });

entity.command('list', 'cli.cmd.list').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
        return 2;
    }
    const opened = await guard(ctx, () => app.open({ path: file }));
    try {
        const entities = await guard(ctx, () => opened.listEntities());
        ctx.emitResult({ revision: opened.revision, entities }, describeEntities(entities));
        return 0;
    } finally {
        opened.close();
    }
});

entity
    .command('move', 'cli.cmd.move')
    .option('--id <entity>', 'cli.opt.id')
    .option('--dx <number>', 'cli.opt.dx')
    .option('--dy <number>', 'cli.opt.dy')
    .option('--to <x,y>', 'cli.opt.to-point')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const target = point(options, 'to');
        const dx = number(options, 'dx');
        const dy = number(options, 'dy');
        const hasOffset = dx !== undefined || dy !== undefined;
        if (id === undefined || (target === undefined && !hasOffset) || (target !== undefined && hasOffset)) {
            ctx.fail('open-cad.cli.move-required', {
                message: '`--id <entity>` plus either `--dx/--dy` or `--to x,y` are required, and the forms cannot be combined',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () => opened.moveEntity({ entityId: id, dx, dy, to: target }));
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEdit(change, 'moved'));
            return 0;
        } finally {
            opened.close();
        }
    });

entity
    .command('update', 'cli.cmd.update')
    .option('--id <entity>', 'cli.opt.id')
    .option('--text <text>', 'cli.opt.text')
    .option('--height <number>', 'cli.opt.height')
    .option('--radius <number>', 'cli.opt.radius')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const content = text(options, 'text');
        const height = number(options, 'height');
        const radius = number(options, 'radius');
        if (id === undefined || (content === undefined && height === undefined && radius === undefined)) {
            ctx.fail('open-cad.cli.update-required', {
                message: '`--id <entity>` and at least one of `--text`, `--height` or `--radius` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () => opened.updateEntity({ entityId: id, content, height, radius }));
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEdit(change, 'updated'));
            return 0;
        } finally {
            opened.close();
        }
    });

const arc = cli.command('arc', 'cli.cmd.arc');

arc
    .command('add', 'cli.cmd.arc-add')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--center <x,y>', 'cli.opt.center')
    .option('--radius <number>', 'cli.opt.radius')
    .option('--start <degrees>', 'cli.opt.start-angle')
    .option('--end <degrees>', 'cli.opt.end-angle')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const center = point(options, 'center');
        const radius = number(options, 'radius');
        const startAngle = number(options, 'start');
        if (center === undefined || radius === undefined || startAngle === undefined) {
            ctx.fail('open-cad.cli.arc-required', {
                message: '`--center x,y`, `--radius <number>` and `--start <degrees>` are required',
            });
            return 2;
        }
        const endAngle = number(options, 'end') ?? startAngle + 360;
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addArc({ layer: text(options, 'layer') ?? '0', center, radius, startAngle, endAngle }),
            );
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

const ellipse = cli.command('ellipse', 'cli.cmd.ellipse');

ellipse
    .command('add', 'cli.cmd.ellipse-add')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--center <x,y>', 'cli.opt.center')
    .option('--rx <number>', 'cli.opt.rx')
    .option('--ry <number>', 'cli.opt.ry')
    .option('--rotation <degrees>', 'cli.opt.rotation')
    .option('--start <degrees>', 'cli.opt.start-angle')
    .option('--end <degrees>', 'cli.opt.end-angle')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const center = point(options, 'center');
        const rx = number(options, 'rx');
        const ry = number(options, 'ry');
        if (center === undefined || rx === undefined || ry === undefined) {
            ctx.fail('open-cad.cli.ellipse-required', {
                message: '`--center x,y`, `--rx <number>` and `--ry <number>` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addEllipse({
                    layer: text(options, 'layer') ?? '0',
                    center,
                    rx,
                    ry,
                    rotation: number(options, 'rotation') ?? 0,
                    startAngle: number(options, 'start') ?? 0,
                    endAngle: number(options, 'end') ?? 360,
                }),
            );
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('transform', 'cli.cmd.transform')
    .option('--ids <list>', 'cli.opt.ids')
    .option('--layer <layer>', 'cli.opt.layer')
    .option('--all', 'cli.opt.all')
    .option('--translate <x,y>', 'cli.opt.translate')
    .option('--rotate <degrees>', 'cli.opt.rotate')
    .option('--scale <factor>', 'cli.opt.scale')
    .option('--mirror <axis>', 'cli.opt.mirror', { enum: ['x', 'y'] })
    .option('--center <x,y>', 'cli.opt.pivot')
    .option('--mirror-at <number>', 'cli.opt.mirror-at')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const ids = idList(options, 'ids');
        const layerId = text(options, 'layer');
        const all = flag(options, 'all');
        const targets = [ids !== undefined, layerId !== undefined, all].filter(Boolean).length;
        if (targets !== 1) {
            ctx.fail('open-cad.cli.target-required', {
                message: 'choose exactly one selection: `--ids entity-1,entity-2`, `--layer <id>` or `--all`',
            });
            return 2;
        }
        const target: TransformTarget = ids !== undefined ? { ids } : layerId !== undefined ? { layer: layerId } : { all: true };

        const translate = point(options, 'translate');
        const rotate = number(options, 'rotate');
        const scale = number(options, 'scale');
        const mirror = text(options, 'mirror') as 'x' | 'y' | undefined;
        const center = point(options, 'center');
        const operations = [translate !== undefined, rotate !== undefined, scale !== undefined, mirror !== undefined].filter(Boolean).length;
        if (operations !== 1) {
            ctx.fail('open-cad.cli.transform-required', {
                message: 'choose exactly one operation: `--translate x,y`, `--rotate <degrees>`, `--scale <factor>` or `--mirror x|y`',
            });
            return 2;
        }
        let operation: TransformOperation;
        if (translate !== undefined) {
            operation = { op: 'translate', dx: translate[0], dy: translate[1] };
        }
        else if (rotate !== undefined) {
            if (center === undefined) {
                ctx.fail('open-cad.cli.center-required', { message: '`--rotate` needs `--center x,y` as the pivot' });
                return 2;
            }
            operation = { op: 'rotate', angle: rotate, cx: center[0], cy: center[1] };
        }
        else if (scale !== undefined) {
            if (center === undefined) {
                ctx.fail('open-cad.cli.center-required', { message: '`--scale` needs `--center x,y` as the pivot' });
                return 2;
            }
            operation = { op: 'scale', factor: scale, cx: center[0], cy: center[1] };
        }
        else {
            operation = { op: 'mirror', axis: mirror ?? 'x', at: number(options, 'mirror-at') ?? 0 };
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.transform({ target, operation }));
            ctx.emitResult(result, describeTransform(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('copy', 'cli.cmd.copy')
    .option('--ids <list>', 'cli.opt.ids')
    .option('--dx <number>', 'cli.opt.dx')
    .option('--dy <number>', 'cli.opt.dy')
    .option('--count <number>', 'cli.opt.count')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const ids = idList(options, 'ids');
        if (ids === undefined) {
            ctx.fail('open-cad.cli.ids-required', { message: '`--ids entity-1,entity-2` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () =>
                opened.copy({
                    ids,
                    dx: number(options, 'dx') ?? 0,
                    dy: number(options, 'dy') ?? 0,
                    count: number(options, 'count') ?? 1,
                }),
            );
            ctx.emitResult(result, describeCopy(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('measure', 'cli.cmd.measure')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.measure(text(options, 'sheet')));
            ctx.emitResult(result, describeMeasure(result));
            return 0;
        } finally {
            opened.close();
        }
    });

const layer = cli.command('layer', 'cli.cmd.layer');

layer
    .command('add', 'cli.cmd.layer-add')
    .option('--name <name>', 'cli.opt.name')
    .option('--color <color>', 'cli.opt.color')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        if (name === undefined) {
            ctx.fail('open-cad.cli.name-required', { message: '`--name <name>` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const created = await guard(ctx, () => opened.addLayer({ name, color: text(options, 'color') }));
            const change = layerChange(opened, created);
            ctx.emitResult(change, describeLayer(change));
            return 0;
        } finally {
            opened.close();
        }
    });

layer
    .command('rename', 'cli.cmd.layer-rename')
    .option('--name <name>', 'cli.opt.name')
    .option('--to <name>', 'cli.opt.to-name')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        const to = text(options, 'to');
        if (name === undefined || to === undefined) {
            ctx.fail('open-cad.cli.layer-rename-required', { message: '`--name <layer>` and `--to <name>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const updated = await guard(ctx, () => opened.updateLayer({ layerId: name, name: to }));
            const change = layerChange(opened, updated);
            ctx.emitResult(change, describeLayer(change));
            return 0;
        } finally {
            opened.close();
        }
    });

layer
    .command('visible', 'cli.cmd.layer-visible')
    .option('--name <name>', 'cli.opt.name')
    .option('--visible <on|off>', 'cli.opt.visible', { enum: ['on', 'off'] })
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        const visible = toggle(options, 'visible');
        if (name === undefined || visible === undefined) {
            ctx.fail('open-cad.cli.layer-visible-required', { message: '`--name <layer>` and `--visible on|off` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const updated = await guard(ctx, () => opened.updateLayer({ layerId: name, visible }));
            const change = layerChange(opened, updated);
            ctx.emitResult(change, describeLayer(change));
            return 0;
        } finally {
            opened.close();
        }
    });

layer
    .command('lock', 'cli.cmd.layer-lock')
    .option('--name <name>', 'cli.opt.name')
    .option('--locked <on|off>', 'cli.opt.locked', { enum: ['on', 'off'] })
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        const locked = toggle(options, 'locked');
        if (name === undefined || locked === undefined) {
            ctx.fail('open-cad.cli.layer-lock-required', { message: '`--name <layer>` and `--locked on|off` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const updated = await guard(ctx, () => opened.updateLayer({ layerId: name, locked }));
            const change = layerChange(opened, updated);
            ctx.emitResult(change, describeLayer(change));
            return 0;
        } finally {
            opened.close();
        }
    });

cli.command('gui', 'cli.cmd.gui').actionCtx(async (_options, _args, ctx) => {
    try {
        const status = await runGui({});
        ctx.emitResult(status, `window ${status.state}`);
        return 0;
    } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        ctx.fail('open-cad.cli.gui-failed', { message });
        return 1;
    }
});

const entry = process.argv[1];
if (entry !== undefined && import.meta.url === pathToFileURL(entry).href) {
    const code = await cli.parse(process.argv);
    process.exitCode = code;
}
