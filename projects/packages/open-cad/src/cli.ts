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
    ArrayPlacement,
    ArrayResult,
    CopyResult,
    DimensionStyle,
    DimensionStyleChange,
    Drawing,
    DrawingStats,
    Entity,
    EntityChange,
    EntityDeletion,
    EntityGroup,
    EntityLayerChange,
    EntityMeasureResult,
    ExplodeResult,
    ExportFormat,
    ExtendResult,
    ExtendSide,
    GroupDeletion,
    HistoryResult,
    ImportLoss,
    InspectCheck,
    InspectReport,
    LayerChange,
    LayerMerge,
    LayerSummary,
    Linetype,
    LinetypeChange,
    MeasureResult,
    OffsetBySideResult,
    OffsetResult,
    OffsetSide,
    OpenCADCapabilities,
    Point,
    PointMeasurement,
    SaveReport,
    SelectionResult,
    SnapMode,
    SnapResult,
    TextStyle,
    TextStyleChange,
    TextMatchResult,
    TextReplaceResult,
    TransformOperation,
    TransformResult,
    TransformTarget,
    TrimResult,
    UcsResult,
    UnitsConversion,
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

const numberList = (options: ParsedOptions, key: string): number[] | undefined => {
    const raw = text(options, key);
    if (raw === undefined) {
        return undefined;
    }
    const values = raw
        .split(',')
        .map((entry) => entry.trim())
        .filter((entry) => entry.length > 0)
        .map((entry) => Number(entry));
    if (values.length === 0 || values.some((value) => !Number.isFinite(value))) {
        return undefined;
    }
    return values;
};

const points = (options: ParsedOptions, key: string): Point[] | undefined => {    const raw = text(options, key);
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

const box = (options: ParsedOptions, key: string): [Point, Point] | undefined => {
    const raw = text(options, key);
    if (raw === undefined) {
        return undefined;
    }
    const parts = raw.split(',');
    if (parts.length !== 4) {
        return undefined;
    }
    const values = parts.map((part) => Number(part.trim()));
    if (values.some((value) => !Number.isFinite(value))) {
        return undefined;
    }
    return [
        [values[0] as number, values[1] as number],
        [values[2] as number, values[3] as number],
    ];
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

function describeArray(result: ArrayResult): string {
    return `arrayed ${result.count} entities revision=${result.revision} created=${result.created.join(',')}`;
}

function describeExplode(result: ExplodeResult): string {
    return `exploded ${result.entity} into ${result.count} lines revision=${result.revision} created=${result.created.join(',')}`;
}

function describeOffset(result: OffsetResult): string {
    return `offset ${result.source} into ${result.entity.id} revision=${result.revision}`;
}

function describeOffsetBySide(result: OffsetBySideResult): string {
    return `offset ${result.source} ${result.side} by ${result.distance} into ${result.entity.id} revision=${result.revision}`;
}

function describeTrim(result: TrimResult): string {
    const kind = result.entity.kind;
    const span = kind.type === 'line' ? ` from=${kind.from[0]},${kind.from[1]} to=${kind.to[0]},${kind.to[1]}` : '';
    return `trimmed ${result.entity.id} with ${result.cutter}${span} revision=${result.revision}`;
}

function describeExtend(result: ExtendResult): string {
    const kind = result.entity.kind;
    const span = kind.type === 'line' ? ` from=${kind.from[0]},${kind.from[1]} to=${kind.to[0]},${kind.to[1]}` : '';
    return `extended ${result.entity.id} ${result.side} to ${result.boundary}${span} revision=${result.revision}`;
}

function describeLayerAssignment(result: EntityLayerChange): string {
    return result.moved
        ? `moved ${result.entity.id} from layer ${result.from} to ${result.to} revision=${result.revision}`
        : `${result.entity.id} is already on layer ${result.to} revision=${result.revision}`;
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

function describeEntityMeasure(result: EntityMeasureResult): string {
    const entries = result.entities.map(
        (entity) =>
            `  ${entity.id} ${entity.kind} layer=${entity.layer}${entity.closed ? ' closed' : ''} length=${entity.length} area=${entity.area}`,
    );
    return [
        `measured ${result.totals.entities} entities revision=${result.revision} units=${result.units}`,
        ...entries,
        `  total: length=${result.totals.length} area=${result.totals.area}`,
    ].join('\n');
}

function describePointMeasure(result: PointMeasurement): string {
    if (result.kind === 'distance') {
        return `distance ${result.distance} dx=${result.dx} dy=${result.dy} bearing=${result.bearing} units=${result.units} revision=${result.revision}`;
    }
    return [
        `angle ${result.angle} units=${result.units} revision=${result.revision}`,
        `  vertex: ${result.vertex[0]},${result.vertex[1]}`,
        `  legs: first=${result.first} second=${result.second} span=${result.span}`,
    ].join('\n');
}

function describeStats(result: DrawingStats): string {
    const layers = result.layers.map(
        (layer) => `  ${layer.id}: entities=${layer.entityCount} length=${layer.length} area=${layer.area}`,
    );
    return [
        `statistics revision=${result.revision} units=${result.units}`,
        ...layers,
        `  total: entities=${result.totals.entities} length=${result.totals.length} area=${result.totals.area}`,
    ].join('\n');
}

function describeTextMatches(result: TextMatchResult): string {
    const lines = [`text match revision=${result.revision} count=${result.count} pattern=${result.pattern}`];
    for (const match of result.matches) {
        lines.push(`  ${match.id} layer=${match.layer} at=${match.at[0]},${match.at[1]} ${match.content}`);
    }
    return lines.join('\n');
}

function describeTextReplace(result: TextReplaceResult): string {
    return `replaced ${result.occurrences} occurrence(s) in ${result.count} text entities revision=${result.revision}`;
}

function describeGroups(groups: readonly EntityGroup[]): string {
    if (groups.length === 0) {
        return 'no entity groups';
    }
    return groups.map((group) => `${group.name}: ${group.entities.join(',')}`).join('\n');
}

function describeGroup(group: EntityGroup, revision: number): string {
    return `entity group ${group.name} entities=${group.entities.length} revision=${revision}`;
}

function describeGroupDeletion(result: GroupDeletion): string {
    return `deleted entity group ${result.deleted} revision=${result.revision} groups=${result.drawing.groups}`;
}

function describeHistory(result: HistoryResult, verb: 'undo' | 'redo'): string {
    return `${verb} revision=${result.revision} entities=${result.drawing.entities} canUndo=${result.canUndo} canRedo=${result.canRedo}`;
}

function describeLayer(change: LayerChange): string {
    const layer = change.layer;
    return `layer ${layer.id} name=${layer.name} color=${layer.color} visible=${layer.visible} locked=${layer.locked} frozen=${layer.frozen} plottable=${layer.plottable} entities=${layer.entityCount} revision=${change.revision}`;
}

function describeMerge(result: LayerMerge): string {
    return `merged layer ${result.from} into ${result.to} moved=${result.moved} revision=${result.revision} layers=${result.drawing.layers.length}`;
}

function describeDimStyles(styles: readonly DimensionStyle[]): string {
    if (styles.length === 0) {
        return 'no dimension styles';
    }
    return styles
        .map((style) => `${style.name} scale=${style.scale} arrow=${style.arrowSize} text=${style.textHeight} precision=${style.precision}`)
        .join('\n');
}

function describeDimStyle(change: DimensionStyleChange): string {
    return `dimension style ${change.style.name} revision=${change.revision}`;
}

function describeTextStyles(styles: readonly TextStyle[]): string {
    if (styles.length === 0) {
        return 'no text styles';
    }
    return styles
        .map((style) => `${style.name} font=${style.font} height=${style.height} width=${style.widthFactor} oblique=${style.oblique}`)
        .join('\n');
}

function describeTextStyle(change: TextStyleChange): string {
    return `text style ${change.style.name} revision=${change.revision}`;
}

function describeLinetype(change: LinetypeChange): string {
    return `line type ${change.linetype.name} pattern=${change.linetype.pattern.join(',')} revision=${change.revision}`;
}

function describeLinetypes(linetypes: readonly Linetype[]): string {
    if (linetypes.length === 0) {
        return 'no line types';
    }
    return linetypes.map((linetype) => `${linetype.name} pattern=${linetype.pattern.join(',')} ${linetype.description}`.trim()).join('\n');
}

function describeSelection(result: SelectionResult): string {
    return [
        `selected ${result.count} entities mode=${result.mode} layer=${result.layer ?? 'any'} revision=${result.revision}`,
        ...result.ids.map((id) => `  ${id}`),
    ].join('\n');
}

function describeSnap(result: SnapResult): string {
    const lines = result.points.map(
        (entry) =>
            `  ${entry.kind} ${entry.point[0]},${entry.point[1]}${entry.distance === undefined ? '' : ` distance=${entry.distance}`} entities=${entry.entities.join(',')}`,
    );
    return [`snap ${result.count} candidates modes=${result.modes.join(',')} revision=${result.revision}`, ...lines].join('\n');
}

function describeUcs(result: UcsResult): string {
    const lines = [
        `ucs ${result.direction} origin=${result.origin[0]},${result.origin[1]} rotation=${result.rotation} revision=${result.revision}`,
        ...result.points.map((point) => `  ${point.from[0]},${point.from[1]} -> ${point.to[0]},${point.to[1]}`),
    ];
    if (result.entities.length > 0) {
        lines.push(`  entities: ${result.entities.join(',')}`);
    }
    return lines.join('\n');
}

function describeUnits(result: UnitsConversion): string {
    return `units ${result.from} -> ${result.to} factor=${result.factor} converted=${result.converted} revision=${result.revision}`;
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
    .option('--format <format>', 'cli.opt.format', { enum: ['dxf', 'dxf-r12', 'obj', 'step', 'stl', 'svg', 'pdf'] })
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
    .command('add-point', 'cli.cmd.add-point')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--at <x,y>', 'cli.opt.point-at')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const at = point(options, 'at');
        if (at === undefined) {
            ctx.fail('open-cad.cli.point-required', { message: '`--at x,y` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addPoint({ layer: text(options, 'layer') ?? '0', at }),
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
    .command('add-spline', 'cli.cmd.add-spline')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--points <list>', 'cli.opt.spline-points')
    .option('--closed', 'cli.opt.closed')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const parsed = points(options, 'points');
        if (parsed === undefined || parsed.length < 2) {
            ctx.fail('open-cad.cli.spline-points-required', {
                message: '`--points "0,0 10,10 20,0 ..."` with at least two fit points is required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addSpline({
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
    .command('add-hatch', 'cli.cmd.add-hatch')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--points <list>', 'cli.opt.points')
    .option('--pattern <name>', 'cli.opt.pattern')
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
                opened.addHatch({
                    layer: text(options, 'layer') ?? '0',
                    points: parsed,
                    pattern: text(options, 'pattern'),
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
    .command('add-rectangle', 'cli.cmd.add-rectangle')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--from <x,y>', 'cli.opt.rectangle-from')
    .option('--to <x,y>', 'cli.opt.rectangle-to')
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
                opened.addRectangle({ layer: text(options, 'layer') ?? '0', from, to }),
            );
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

entity
    .command('add-polygon', 'cli.cmd.add-polygon')
    .option('--layer <layer>', 'cli.opt.layer', { default: '0' })
    .option('--center <x,y>', 'cli.opt.polygon-center')
    .option('--radius <number>', 'cli.opt.polygon-radius')
    .option('--sides <n>', 'cli.opt.sides')
    .option('--rotation <deg>', 'cli.opt.polygon-rotation')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const center = point(options, 'center');
        const radius = number(options, 'radius');
        const sides = number(options, 'sides');
        if (center === undefined || radius === undefined || sides === undefined) {
            ctx.fail('open-cad.cli.polygon-required', {
                message: '`--center x,y`, `--radius <number>` and `--sides <n>` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addPolygon({
                    layer: text(options, 'layer') ?? '0',
                    center,
                    radius,
                    sides,
                    rotation: number(options, 'rotation'),
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
    .command('add-radial-dimension', 'cli.cmd.add-radial-dimension')
    .option('--layer <layer>', 'cli.opt.layer', { default: 'dimensions' })
    .option('--center <x,y>', 'cli.opt.center')
    .option('--radius <number>', 'cli.opt.radius')
    .option('--angle <degrees>', 'cli.opt.angle')
    .option('--offset <number>', 'cli.opt.offset')
    .option('--diameter', 'cli.opt.diameter')
    .option('--text <text>', 'cli.opt.text')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const center = point(options, 'center');
        const radius = number(options, 'radius');
        const angle = number(options, 'angle');
        if (center === undefined || radius === undefined || angle === undefined) {
            ctx.fail('open-cad.cli.radial-dimension-required', {
                message: '`--center x,y`, `--radius <number>` and `--angle <degrees>` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addRadialDimension({
                    layer: text(options, 'layer') ?? 'dimensions',
                    center,
                    radius,
                    angle,
                    offset: number(options, 'offset') ?? 0,
                    diameter: flag(options, 'diameter'),
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
    .command('add-ordinate-dimension', 'cli.cmd.add-ordinate-dimension')
    .option('--layer <layer>', 'cli.opt.layer', { default: 'dimensions' })
    .option('--origin <x,y>', 'cli.opt.origin')
    .option('--feature <x,y>', 'cli.opt.feature')
    .option('--axis <axis>', 'cli.opt.axis', { enum: ['x', 'y'] })
    .option('--offset <number>', 'cli.opt.offset')
    .option('--text <text>', 'cli.opt.text')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const origin = point(options, 'origin');
        const feature = point(options, 'feature');
        const axis = text(options, 'axis') as 'x' | 'y' | undefined;
        if (origin === undefined || feature === undefined || axis === undefined) {
            ctx.fail('open-cad.cli.ordinate-dimension-required', {
                message: '`--origin x,y`, `--feature x,y` and `--axis x|y` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addOrdinateDimension({
                    layer: text(options, 'layer') ?? 'dimensions',
                    origin,
                    feature,
                    axis,
                    offset: number(options, 'offset') ?? 0,
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
    .command('add-angular-dimension', 'cli.cmd.add-angular-dimension')
    .option('--center <point>', 'cli.opt.center')
    .option('--first <point>', 'cli.opt.first')
    .option('--second <point>', 'cli.opt.second')
    .option('--offset <number>', 'cli.opt.offset')
    .option('--text <text>', 'cli.opt.text')
    .option('--layer <layer>', 'cli.opt.dimension-layer')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const center = point(options, 'center');
        const first = point(options, 'first');
        const second = point(options, 'second');
        if (center === undefined || first === undefined || second === undefined) {
            ctx.fail('open-cad.cli.angular-dimension-required', {
                message: '`--center x,y`, `--first x,y` and `--second x,y` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () =>
                opened.addAngularDimension({
                    layer: text(options, 'layer') ?? 'dimensions',
                    center,
                    first,
                    second,
                    offset: number(options, 'offset'),
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

entity
    .command('set-layer', 'cli.cmd.set-layer')
    .option('--id <entity>', 'cli.opt.id')
    .option('--to <layer>', 'cli.opt.target-layer')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const layer = text(options, 'to');
        if (id === undefined || layer === undefined) {
            ctx.fail('open-cad.cli.set-layer-required', { message: '`--id <entity>` and `--to <layer>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.assignEntityLayer({ entityId: id, layer }));
            ctx.emitResult(result, describeLayerAssignment(result));
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

arc
    .command('update', 'cli.cmd.arc-update')
    .option('--id <entity>', 'cli.opt.id')
    .option('--radius <number>', 'cli.opt.radius')
    .option('--start <degrees>', 'cli.opt.start-angle')
    .option('--end <degrees>', 'cli.opt.end-angle')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const radius = number(options, 'radius');
        const startAngle = number(options, 'start');
        const endAngle = number(options, 'end');
        if (id === undefined || (radius === undefined && startAngle === undefined && endAngle === undefined)) {
            ctx.fail('open-cad.cli.arc-update-required', {
                message: '`--id <entity>` and at least one of `--radius`, `--start` or `--end` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () => opened.updateArc({ entityId: id, radius, startAngle, endAngle }));
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEdit(change, 'updated'));
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

const polyline = cli.command('polyline', 'cli.cmd.polyline');

polyline
    .command('update', 'cli.cmd.polyline-update')
    .option('--id <entity>', 'cli.opt.id')
    .option('--points <list>', 'cli.opt.points')
    .option('--closed <on|off>', 'cli.opt.closed-state', { enum: ['on', 'off'] })
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const parsed = points(options, 'points');
        const closed = toggle(options, 'closed');
        if (id === undefined || (parsed === undefined && closed === undefined)) {
            ctx.fail('open-cad.cli.polyline-update-required', {
                message: '`--id <entity>` and `--points "x,y ..."` or `--closed on|off` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () => opened.updatePolyline({ entityId: id, points: parsed, closed }));
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEdit(change, 'updated'));
            return 0;
        } finally {
            opened.close();
        }
    });

polyline
    .command('insert', 'cli.cmd.polyline-insert')
    .option('--id <entity>', 'cli.opt.id')
    .option('--index <number>', 'cli.opt.vertex-index')
    .option('--at <x,y>', 'cli.opt.vertex-at')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const index = number(options, 'index');
        const at = point(options, 'at');
        if (id === undefined || index === undefined || !Number.isInteger(index) || index < 0 || at === undefined) {
            ctx.fail('open-cad.cli.polyline-insert-required', {
                message: '`--id <entity>`, `--index <number>` and `--at x,y` are required, with a non-negative integer index',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () => opened.updatePolyline({ entityId: id, insertAt: index, insertPoint: at }));
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEdit(change, 'updated'));
            return 0;
        } finally {
            opened.close();
        }
    });

polyline
    .command('remove-vertex', 'cli.cmd.polyline-remove')
    .option('--id <entity>', 'cli.opt.id')
    .option('--index <number>', 'cli.opt.vertex-index')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const index = number(options, 'index');
        if (id === undefined || index === undefined || !Number.isInteger(index) || index < 0) {
            ctx.fail('open-cad.cli.polyline-remove-required', {
                message: '`--id <entity>` and a non-negative integer `--index <number>` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () => opened.updatePolyline({ entityId: id, removeAt: index }));
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEdit(change, 'updated'));
            return 0;
        } finally {
            opened.close();
        }
    });

polyline
    .command('move-vertex', 'cli.cmd.polyline-move')
    .option('--id <entity>', 'cli.opt.id')
    .option('--index <number>', 'cli.opt.vertex-index')
    .option('--at <x,y>', 'cli.opt.vertex-move-at')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const index = number(options, 'index');
        const at = point(options, 'at');
        if (id === undefined || index === undefined || !Number.isInteger(index) || index < 0 || at === undefined) {
            ctx.fail('open-cad.cli.polyline-move-required', {
                message: '`--id <entity>`, `--index <number>` and `--at x,y` are required, with a non-negative integer index',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () => opened.updatePolyline({ entityId: id, moveAt: index, moveTo: at }));
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEdit(change, 'updated'));
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
    .command('array', 'cli.cmd.array')
    .option('--ids <list>', 'cli.opt.ids')
    .option('--rows <count>', 'cli.opt.rows')
    .option('--columns <count>', 'cli.opt.columns')
    .option('--row-spacing <number>', 'cli.opt.row-spacing')
    .option('--column-spacing <number>', 'cli.opt.column-spacing')
    .option('--center <x,y>', 'cli.opt.center')
    .option('--count <count>', 'cli.opt.count')
    .option('--angle <degrees>', 'cli.opt.total-angle')
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
        const rows = number(options, 'rows');
        const columns = number(options, 'columns');
        const rowSpacing = number(options, 'row-spacing');
        const columnSpacing = number(options, 'column-spacing');
        const center = point(options, 'center');
        const count = number(options, 'count');
        const totalAngle = number(options, 'angle');
        const rectangular = rows !== undefined || columns !== undefined || rowSpacing !== undefined || columnSpacing !== undefined;
        const polar = center !== undefined || count !== undefined || totalAngle !== undefined;
        if (rectangular === polar) {
            ctx.fail('open-cad.cli.array-required', {
                message: 'choose exactly one array mode: `--rows/--columns` or `--center/--count/--angle`',
            });
            return 2;
        }
        let placement: ArrayPlacement;
        if (rectangular) {
            if (rows === undefined || columns === undefined) {
                ctx.fail('open-cad.cli.array-required', {
                    message: '`--rows <count>` and `--columns <count>` are required for a rectangular array',
                });
                return 2;
            }
            placement = {
                mode: 'rectangular',
                rows,
                columns,
                rowSpacing: rowSpacing ?? 0,
                columnSpacing: columnSpacing ?? 0,
            };
        }
        else {
            if (center === undefined || count === undefined || totalAngle === undefined) {
                ctx.fail('open-cad.cli.array-required', {
                    message: '`--center x,y`, `--count <count>` and `--angle <degrees>` are required for a polar array',
                });
                return 2;
            }
            placement = { mode: 'polar', center, count, totalAngle };
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.array({ ids, placement }));
            ctx.emitResult(result, describeArray(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('explode', 'cli.cmd.explode')
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
            const result = await guard(ctx, () => opened.explode(id));
            ctx.emitResult(result, describeExplode(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('offset', 'cli.cmd.offset')
    .option('--id <entity>', 'cli.opt.id')
    .option('--distance <number>', 'cli.opt.distance')
    .option('--side <side>', 'cli.opt.offset-side', { enum: ['left', 'right', 'outside', 'inside'] })
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const distance = number(options, 'distance');
        const side = text(options, 'side') as OffsetSide | undefined;
        if (id === undefined || distance === undefined) {
            ctx.fail('open-cad.cli.offset-required', { message: '`--id <entity>` and `--distance <number>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            if (side === undefined) {
                const result = await guard(ctx, () => opened.offset({ entityId: id, distance }));
                ctx.emitResult(result, describeOffset(result));
            }
            else {
                const result = await guard(ctx, () => opened.offsetBySide({ entityId: id, distance, side }));
                ctx.emitResult(result, describeOffsetBySide(result));
            }
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('trim', 'cli.cmd.trim')
    .option('--id <entity>', 'cli.opt.id')
    .option('--cutter <entity>', 'cli.opt.cutter')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const cutter = text(options, 'cutter');
        if (id === undefined || cutter === undefined) {
            ctx.fail('open-cad.cli.trim-required', { message: '`--id <entity>` and `--cutter <entity>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.trim(id, cutter));
            ctx.emitResult(result, describeTrim(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('extend', 'cli.cmd.extend')
    .option('--id <line>', 'cli.opt.id')
    .option('--to <entity>', 'cli.opt.extend-to')
    .option('--side <side>', 'cli.opt.extend-side', { enum: ['end', 'start'] })
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const id = text(options, 'id');
        const boundary = text(options, 'to');
        const side = (text(options, 'side') as ExtendSide | undefined) ?? 'end';
        if (id === undefined || boundary === undefined) {
            ctx.fail('open-cad.cli.extend-required', { message: '`--id <line>` and `--to <entity>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.extend(id, boundary, side));
            ctx.emitResult(result, describeExtend(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('fillet', 'cli.cmd.fillet')
    .option('--first <entity>', 'cli.opt.first')
    .option('--second <entity>', 'cli.opt.second')
    .option('--radius <number>', 'cli.opt.radius')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const first = text(options, 'first');
        const second = text(options, 'second');
        const radius = number(options, 'radius');
        if (first === undefined || second === undefined || radius === undefined) {
            ctx.fail('open-cad.cli.corner-required', {
                message: '`--first <entity>`, `--second <entity>` and `--radius <number>` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () => opened.fillet(first, second, radius));
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('chamfer', 'cli.cmd.chamfer')
    .option('--first <entity>', 'cli.opt.first')
    .option('--second <entity>', 'cli.opt.second')
    .option('--distance-a <number>', 'cli.opt.distance-a')
    .option('--distance-b <number>', 'cli.opt.distance-b')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const first = text(options, 'first');
        const second = text(options, 'second');
        const distanceA = number(options, 'distance-a');
        const distanceB = number(options, 'distance-b') ?? distanceA;
        if (first === undefined || second === undefined || distanceA === undefined || distanceB === undefined) {
            ctx.fail('open-cad.cli.corner-required', {
                message: '`--first <entity>`, `--second <entity>` and `--distance-a <number>` are required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entity = await guard(ctx, () => opened.chamfer(first, second, distanceA, distanceB));
            const change = entityChange(opened, entity);
            ctx.emitResult(change, describeEntity(change));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('measure', 'cli.cmd.measure')
    .option('--ids <list>', 'cli.opt.ids')
    .option('--sheet <sheet>', 'cli.opt.sheet')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const ids = idList(options, 'ids');
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            if (ids === undefined) {
                const result = await guard(ctx, () => opened.measure(text(options, 'sheet')));
                ctx.emitResult(result, describeMeasure(result));
            }
            else {
                const result = await guard(ctx, () => opened.measureEntities(ids));
                ctx.emitResult(result, describeEntityMeasure(result));
            }
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('measure-points', 'cli.cmd.measure-points')
    .option('--points <list>', 'cli.opt.points-list')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const coordinates = points(options, 'points');
        if (coordinates === undefined || (coordinates.length !== 2 && coordinates.length !== 3)) {
            ctx.fail('open-cad.cli.measure-points-required', {
                message: '`--points "x,y x,y ..."` with exactly 2 or 3 points is required',
            });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.measurePoints(coordinates));
            ctx.emitResult(result, describePointMeasure(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('stats', 'cli.cmd.stats')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.stats());
            ctx.emitResult(result, describeStats(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('select', 'cli.cmd.select')
    .option('--box <x1,y1,x2,y2>', 'cli.opt.box')
    .option('--crossing', 'cli.opt.crossing')
    .option('--layer <layer>', 'cli.opt.select-layer')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const window = box(options, 'box');
        if (window === undefined) {
            ctx.fail('open-cad.cli.box-required', { message: '`--box x1,y1,x2,y2` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () =>
                opened.select({
                    window,
                    mode: flag(options, 'crossing') ? 'crossing' : 'window',
                    layer: text(options, 'layer'),
                }),
            );
            ctx.emitResult(result, describeSelection(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('snap', 'cli.cmd.snap')
    .option('--modes <list>', 'cli.opt.snap-modes')
    .option('--near <x,y>', 'cli.opt.near')
    .option('--tolerance <number>', 'cli.opt.tolerance')
    .option('--layer <layer>', 'cli.opt.select-layer')
    .option('--ids <list>', 'cli.opt.ids')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const near = point(options, 'near');
        const modes = idList(options, 'modes')?.map((entry) => entry as SnapMode);
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () =>
                opened.snap({
                    modes,
                    point: near,
                    tolerance: number(options, 'tolerance'),
                    layer: text(options, 'layer'),
                    ids: idList(options, 'ids'),
                }),
            );
            ctx.emitResult(result, describeSnap(result));
            return 0;
        } finally {
            opened.close();
        }
    });

cli
    .command('ucs', 'cli.cmd.ucs')
    .option('--origin <x,y>', 'cli.opt.ucs-origin')
    .option('--rotation <degrees>', 'cli.opt.ucs-rotation')
    .option('--direction <direction>', 'cli.opt.ucs-direction', { enum: ['to-world', 'to-ucs'] })
    .option('--points <list>', 'cli.opt.points-list')
    .option('--ids <list>', 'cli.opt.ids')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const coordinates = points(options, 'points');
        const ids = idList(options, 'ids');
        if (coordinates === undefined && ids === undefined) {
            ctx.fail('open-cad.cli.ucs-required', { message: '`--points "x,y ..."`, `--ids`, or both are required' });
            return 2;
        }
        const origin: Point = point(options, 'origin') ?? [0, 0];
        const direction = (text(options, 'direction') as 'to-world' | 'to-ucs' | undefined) ?? 'to-world';
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () =>
                opened.ucs({
                    origin,
                    rotation: number(options, 'rotation') ?? 0,
                    direction,
                    points: coordinates,
                    ids,
                }),
            );
            ctx.emitResult(result, describeUcs(result));
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

layer
    .command('freeze', 'cli.cmd.layer-freeze')
    .option('--name <name>', 'cli.opt.name')
    .option('--frozen <on|off>', 'cli.opt.frozen', { enum: ['on', 'off'] })
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        const frozen = toggle(options, 'frozen');
        if (name === undefined || frozen === undefined) {
            ctx.fail('open-cad.cli.layer-freeze-required', { message: '`--name <layer>` and `--frozen on|off` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const updated = await guard(ctx, () => opened.updateLayer({ layerId: name, frozen }));
            const change = layerChange(opened, updated);
            ctx.emitResult(change, describeLayer(change));
            return 0;
        } finally {
            opened.close();
        }
    });

layer
    .command('plot', 'cli.cmd.layer-plot')
    .option('--name <name>', 'cli.opt.name')
    .option('--plottable <on|off>', 'cli.opt.plottable', { enum: ['on', 'off'] })
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        const plottable = toggle(options, 'plottable');
        if (name === undefined || plottable === undefined) {
            ctx.fail('open-cad.cli.layer-plot-required', { message: '`--name <layer>` and `--plottable on|off` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const updated = await guard(ctx, () => opened.updateLayer({ layerId: name, plottable }));
            const change = layerChange(opened, updated);
            ctx.emitResult(change, describeLayer(change));
            return 0;
        } finally {
            opened.close();
        }
    });

layer
    .command('set', 'cli.cmd.layer-set')
    .option('--name <name>', 'cli.opt.name')
    .option('--linetype <name>', 'cli.opt.linetype')
    .option('--lineweight <mm>', 'cli.opt.lineweight')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        const linetype = text(options, 'linetype');
        const lineweight = number(options, 'lineweight');
        if (name === undefined || (linetype === undefined && lineweight === undefined)) {
            ctx.fail('open-cad.cli.layer-set-required', { message: '`--name <layer>` and `--linetype` or `--lineweight` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const updated = await guard(ctx, () => opened.updateLayer({ layerId: name, linetype, lineweight }));
            const change = layerChange(opened, updated);
            ctx.emitResult(change, describeLayer(change));
            return 0;
        } finally {
            opened.close();
        }
    });

layer
    .command('merge', 'cli.cmd.layer-merge')
    .option('--from <layer>', 'cli.opt.layer-from')
    .option('--to <layer>', 'cli.opt.layer-to')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const from = text(options, 'from');
        const to = text(options, 'to');
        if (from === undefined || to === undefined) {
            ctx.fail('open-cad.cli.layer-merge-required', { message: '`--from <layer>` and `--to <layer>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.mergeLayer({ from, to }));
            ctx.emitResult(result, describeMerge(result));
            return 0;
        } finally {
            opened.close();
        }
    });

layer
    .command('delete', 'cli.cmd.layer-delete')
    .option('--name <name>', 'cli.opt.name')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        if (name === undefined) {
            ctx.fail('open-cad.cli.layer-delete-required', { message: '`--name <layer>` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const deletion = await guard(ctx, () => opened.deleteLayer(name));
            ctx.emitResult(deletion, `deleted layer ${deletion.deleted} revision=${deletion.revision} layers=${deletion.drawing.layers.length}`);
            return 0;
        } finally {
            opened.close();
        }
    });

const dimstyle = cli.command('dimstyle', 'cli.cmd.dimstyle');

dimstyle.command('list', 'cli.cmd.dimstyle-list').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
        return 2;
    }
    const opened = await guard(ctx, () => app.open({ path: file }));
    try {
        const styles = await guard(ctx, () => opened.listDimStyles());
        ctx.emitResult({ revision: opened.revision, dimStyles: styles }, describeDimStyles(styles));
        return 0;
    } finally {
        opened.close();
    }
});

dimstyle
    .command('add', 'cli.cmd.dimstyle-add')
    .option('--name <name>', 'cli.opt.style-name')
    .option('--scale <number>', 'cli.opt.scale')
    .option('--arrow-size <number>', 'cli.opt.arrow-size')
    .option('--text-height <number>', 'cli.opt.text-height')
    .option('--extension <number>', 'cli.opt.extension')
    .option('--origin-offset <number>', 'cli.opt.origin-offset')
    .option('--precision <number>', 'cli.opt.precision')
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
        const precision = number(options, 'precision');
        if (precision !== undefined && (!Number.isInteger(precision) || precision < 0 || precision > 8)) {
            ctx.fail('open-cad.cli.precision-invalid', { message: '`--precision` must be an integer from 0 to 8' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const style = await guard(ctx, () =>
                opened.addDimStyle({
                    name,
                    scale: number(options, 'scale'),
                    arrowSize: number(options, 'arrow-size'),
                    textHeight: number(options, 'text-height'),
                    extension: number(options, 'extension'),
                    originOffset: number(options, 'origin-offset'),
                    precision,
                }),
            );
            const change: DimensionStyleChange = { revision: opened.revision, drawing: opened.summary(), style };
            ctx.emitResult(change, describeDimStyle(change));
            return 0;
        } finally {
            opened.close();
        }
    });

dimstyle
    .command('update', 'cli.cmd.dimstyle-update')
    .option('--name <name>', 'cli.opt.style-name')
    .option('--to <name>', 'cli.opt.to-name')
    .option('--scale <number>', 'cli.opt.scale')
    .option('--arrow-size <number>', 'cli.opt.arrow-size')
    .option('--text-height <number>', 'cli.opt.text-height')
    .option('--extension <number>', 'cli.opt.extension')
    .option('--origin-offset <number>', 'cli.opt.origin-offset')
    .option('--precision <number>', 'cli.opt.precision')
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
        const precision = number(options, 'precision');
        if (precision !== undefined && (!Number.isInteger(precision) || precision < 0 || precision > 8)) {
            ctx.fail('open-cad.cli.precision-invalid', { message: '`--precision` must be an integer from 0 to 8' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const style = await guard(ctx, () =>
                opened.updateDimStyle({
                    styleId: name,
                    name: text(options, 'to'),
                    scale: number(options, 'scale'),
                    arrowSize: number(options, 'arrow-size'),
                    textHeight: number(options, 'text-height'),
                    extension: number(options, 'extension'),
                    originOffset: number(options, 'origin-offset'),
                    precision,
                }),
            );
            const change: DimensionStyleChange = { revision: opened.revision, drawing: opened.summary(), style };
            ctx.emitResult(change, describeDimStyle(change));
            return 0;
        } finally {
            opened.close();
        }
    });

dimstyle
    .command('delete', 'cli.cmd.dimstyle-delete')
    .option('--name <name>', 'cli.opt.style-name')
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
            const deletion = await guard(ctx, () => opened.deleteDimStyle(name));
            ctx.emitResult(deletion, `deleted dimension style ${deletion.deleted} revision=${deletion.revision}`);
            return 0;
        } finally {
            opened.close();
        }
    });

const textstyle = cli.command('textstyle', 'cli.cmd.textstyle');

textstyle.command('list', 'cli.cmd.textstyle-list').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
        return 2;
    }
    const opened = await guard(ctx, () => app.open({ path: file }));
    try {
        const styles = await guard(ctx, () => opened.listTextStyles());
        ctx.emitResult({ revision: opened.revision, textStyles: styles }, describeTextStyles(styles));
        return 0;
    } finally {
        opened.close();
    }
});

textstyle
    .command('add', 'cli.cmd.textstyle-add')
    .option('--name <name>', 'cli.opt.text-style-name')
    .option('--font <name>', 'cli.opt.font')
    .option('--height <number>', 'cli.opt.fixed-height')
    .option('--width-factor <number>', 'cli.opt.width-factor')
    .option('--oblique <degrees>', 'cli.opt.oblique')
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
            const style = await guard(ctx, () =>
                opened.addTextStyle({
                    name,
                    font: text(options, 'font'),
                    height: number(options, 'height'),
                    widthFactor: number(options, 'width-factor'),
                    oblique: number(options, 'oblique'),
                }),
            );
            const change: TextStyleChange = { revision: opened.revision, drawing: opened.summary(), style };
            ctx.emitResult(change, describeTextStyle(change));
            return 0;
        } finally {
            opened.close();
        }
    });

textstyle
    .command('update', 'cli.cmd.textstyle-update')
    .option('--name <name>', 'cli.opt.text-style-name')
    .option('--to <name>', 'cli.opt.to-name')
    .option('--font <name>', 'cli.opt.font')
    .option('--height <number>', 'cli.opt.fixed-height')
    .option('--width-factor <number>', 'cli.opt.width-factor')
    .option('--oblique <degrees>', 'cli.opt.oblique')
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
            const style = await guard(ctx, () =>
                opened.updateTextStyle({
                    styleId: name,
                    name: text(options, 'to'),
                    font: text(options, 'font'),
                    height: number(options, 'height'),
                    widthFactor: number(options, 'width-factor'),
                    oblique: number(options, 'oblique'),
                }),
            );
            const change: TextStyleChange = { revision: opened.revision, drawing: opened.summary(), style };
            ctx.emitResult(change, describeTextStyle(change));
            return 0;
        } finally {
            opened.close();
        }
    });

textstyle
    .command('delete', 'cli.cmd.textstyle-delete')
    .option('--name <name>', 'cli.opt.text-style-name')
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
            const deletion = await guard(ctx, () => opened.deleteTextStyle(name));
            ctx.emitResult(deletion, `deleted text style ${deletion.deleted} revision=${deletion.revision}`);
            return 0;
        } finally {
            opened.close();
        }
    });

const linetype = cli.command('linetype', 'cli.cmd.linetype');

linetype.command('list', 'cli.cmd.linetype-list').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
        return 2;
    }
    const opened = await guard(ctx, () => app.open({ path: file }));
    try {
        const entries = await guard(ctx, () => opened.listLinetypes());
        ctx.emitResult({ revision: opened.revision, linetypes: entries }, describeLinetypes(entries));
        return 0;
    } finally {
        opened.close();
    }
});

linetype
    .command('add', 'cli.cmd.linetype-add')
    .option('--name <name>', 'cli.opt.linetype-name')
    .option('--description <text>', 'cli.opt.linetype-description')
    .option('--pattern <list>', 'cli.opt.linetype-pattern')
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
            const entry = await guard(ctx, () =>
                opened.addLinetype({
                    name,
                    description: text(options, 'description'),
                    pattern: numberList(options, 'pattern'),
                }),
            );
            const change: LinetypeChange = { revision: opened.revision, drawing: opened.summary(), linetype: entry };
            ctx.emitResult(change, describeLinetype(change));
            return 0;
        } finally {
            opened.close();
        }
    });

linetype
    .command('update', 'cli.cmd.linetype-update')
    .option('--name <name>', 'cli.opt.linetype-name')
    .option('--to <name>', 'cli.opt.to-name')
    .option('--description <text>', 'cli.opt.linetype-description')
    .option('--pattern <list>', 'cli.opt.linetype-pattern')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        const to = text(options, 'to');
        const description = text(options, 'description');
        const pattern = numberList(options, 'pattern');
        if (name === undefined || (to === undefined && description === undefined && pattern === undefined)) {
            ctx.fail('open-cad.cli.linetype-update-required', { message: '`--name <name>` and one of `--to`, `--description` or `--pattern` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const entry = await guard(ctx, () => opened.updateLinetype({ linetypeId: name, name: to, description, pattern }));
            const change: LinetypeChange = { revision: opened.revision, drawing: opened.summary(), linetype: entry };
            ctx.emitResult(change, describeLinetype(change));
            return 0;
        } finally {
            opened.close();
        }
    });

linetype
    .command('delete', 'cli.cmd.linetype-delete')
    .option('--name <name>', 'cli.opt.linetype-name')
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
            const deletion = await guard(ctx, () => opened.deleteLinetype(name));
            ctx.emitResult(deletion, `deleted line type ${deletion.deleted} revision=${deletion.revision}`);
            return 0;
        } finally {
            opened.close();
        }
    });

const group = cli.command('group', 'cli.cmd.group');

group.command('list', 'cli.cmd.group-list').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
        return 2;
    }
    const opened = await guard(ctx, () => app.open({ path: file }));
    try {
        const groups = await guard(ctx, () => opened.listGroups());
        ctx.emitResult({ revision: opened.revision, groups }, describeGroups(groups));
        return 0;
    } finally {
        opened.close();
    }
});

group
    .command('create', 'cli.cmd.group-create')
    .option('--name <name>', 'cli.opt.group-name')
    .option('--ids <list>', 'cli.opt.ids')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const name = text(options, 'name');
        const entities = idList(options, 'ids');
        if (name === undefined || entities === undefined) {
            ctx.fail('open-cad.cli.group-create-required', { message: '`--name <name>` and `--ids <list>` are required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const created = await guard(ctx, () => opened.createGroup({ name, entities }));
            ctx.emitResult({ revision: opened.revision, drawing: opened.summary(), group: created }, describeGroup(created, opened.revision));
            return 0;
        } finally {
            opened.close();
        }
    });

group
    .command('delete', 'cli.cmd.group-delete')
    .option('--name <name>', 'cli.opt.group-name')
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
            const deletion = await guard(ctx, () => opened.deleteGroup(name));
            ctx.emitResult(deletion, describeGroupDeletion(deletion));
            return 0;
        } finally {
            opened.close();
        }
    });

const groupMemberCommand = (name: string, description: string, add: boolean) => {
    group
        .command(name, description)
        .option('--name <name>', 'cli.opt.group-name')
        .option('--ids <list>', 'cli.opt.ids')
        .actionCtx(async (options, _args, ctx) => {
            const file = input(options);
            if (file === undefined) {
                ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
                return 2;
            }
            const groupId = text(options, 'name');
            const entities = idList(options, 'ids');
            if (groupId === undefined || entities === undefined) {
                ctx.fail('open-cad.cli.group-members-required', { message: '`--name <name>` and `--ids <list>` are required' });
                return 2;
            }
            const opened = await guard(ctx, () => app.open({ path: file }));
            try {
                const updated = await guard(ctx, () =>
                    add
                        ? opened.addGroupMembers({ groupId, entities })
                        : opened.removeGroupMembers({ groupId, entities }),
                );
                ctx.emitResult({ revision: opened.revision, drawing: opened.summary(), group: updated }, describeGroup(updated, opened.revision));
                return 0;
            } finally {
                opened.close();
            }
        });
};

groupMemberCommand('add', 'cli.cmd.group-add', true);
groupMemberCommand('remove', 'cli.cmd.group-remove', false);

const textCommand = cli.command('text', 'cli.cmd.text');

textCommand
    .command('find', 'cli.cmd.text-find')
    .option('--pattern <text>', 'cli.opt.pattern')
    .option('--match-case', 'cli.opt.match-case')
    .option('--layer <layer>', 'cli.opt.select-layer')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const pattern = text(options, 'pattern');
        if (pattern === undefined) {
            ctx.fail('open-cad.cli.pattern-required', { message: '`--pattern <text>` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () =>
                opened.findText({ pattern, matchCase: flag(options, 'match-case'), layer: text(options, 'layer') }),
            );
            ctx.emitResult(result, describeTextMatches(result));
            return 0;
        } finally {
            opened.close();
        }
    });

textCommand
    .command('replace', 'cli.cmd.text-replace')
    .option('--pattern <text>', 'cli.opt.pattern')
    .option('--to <text>', 'cli.opt.replacement')
    .option('--match-case', 'cli.opt.match-case')
    .option('--layer <layer>', 'cli.opt.select-layer')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const pattern = text(options, 'pattern');
        if (pattern === undefined) {
            ctx.fail('open-cad.cli.pattern-required', { message: '`--pattern <text>` is required' });
            return 2;
        }
        const replacement = text(options, 'to');
        if (replacement === undefined) {
            ctx.fail('open-cad.cli.replacement-required', { message: '`--to <text>` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () =>
                opened.replaceText({ pattern, replacement, matchCase: flag(options, 'match-case'), layer: text(options, 'layer') }),
            );
            ctx.emitResult(result, describeTextReplace(result));
            return 0;
        } finally {
            opened.close();
        }
    });

const properties = cli.command('properties', 'cli.cmd.properties');

properties.command('show', 'cli.cmd.properties-show').actionCtx(async (options, _args, ctx) => {
    const file = input(options);
    if (file === undefined) {
        ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
        return 2;
    }
    const opened = await guard(ctx, () => app.open({ path: file }));
    try {
        const result = { revision: opened.revision, title: opened.title, properties: opened.properties };
        ctx.emitResult(
            result,
            [
                `properties revision=${result.revision}`,
                `  title: ${result.title ?? '(untitled)'}`,
                `  author: ${result.properties.author}`,
                `  company: ${result.properties.company}`,
                `  project: ${result.properties.project}`,
                `  description: ${result.properties.description}`,
                `  keywords: ${result.properties.keywords}`,
            ].join('\n'),
        );
        return 0;
    } finally {
        opened.close();
    }
});

properties
    .command('set', 'cli.cmd.properties-set')
    .option('--title <title>', 'cli.opt.title')
    .option('--author <text>', 'cli.opt.author')
    .option('--description <text>', 'cli.opt.description')
    .option('--company <text>', 'cli.opt.company')
    .option('--project <text>', 'cli.opt.project')
    .option('--keywords <text>', 'cli.opt.keywords')
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const request = {
            title: text(options, 'title'),
            author: text(options, 'author'),
            description: text(options, 'description'),
            company: text(options, 'company'),
            project: text(options, 'project'),
            keywords: text(options, 'keywords'),
        };
        if (Object.values(request).every((value) => value === undefined)) {
            ctx.fail('open-cad.cli.properties-required', { message: 'pass at least one property to set' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const updated = await guard(ctx, () => opened.updateProperties(request));
            const change = { revision: opened.revision, drawing: opened.summary(), title: opened.title, properties: updated };
            ctx.emitResult(change, `updated properties revision=${change.revision} title=${change.title ?? '(untitled)'}`);
            return 0;
        } finally {
            opened.close();
        }
    });

const units = cli.command('units', 'cli.cmd.units');

units
    .command('convert', 'cli.cmd.units-convert')
    .option('--to <unit>', 'cli.opt.to-unit', { enum: ['mm', 'inch'] })
    .actionCtx(async (options, _args, ctx) => {
        const file = input(options);
        if (file === undefined) {
            ctx.fail('open-cad.cli.file-required', { message: 'a drawing file is required' });
            return 2;
        }
        const to = text(options, 'to') as 'mm' | 'inch' | undefined;
        if (to === undefined) {
            ctx.fail('open-cad.cli.units-required', { message: '`--to mm|inch` is required' });
            return 2;
        }
        const opened = await guard(ctx, () => app.open({ path: file }));
        try {
            const result = await guard(ctx, () => opened.convertUnits({ to }));
            ctx.emitResult(result, describeUnits(result));
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
