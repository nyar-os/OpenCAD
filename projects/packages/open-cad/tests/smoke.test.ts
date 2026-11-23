/**
 * Smoke tests for the drawing surface.
 *
 * They assert shell semantics when the addon is absent and the real document
 * core when a built addon is installed: create, edit, save/open round trip,
 * inspection and SVG/DXF export.
 */

import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import { drawingFormats, formatFromPath, formatInfo, OpenCAD, OpenCADError } from '../src/index.js';
import type { DrawingInput } from '../src/index.js';
import { nativeBinding } from '../src/native.js';
import * as service from '../src/service.js';

const installed = (() => {
    try {
        return nativeBinding() !== undefined;
    } catch {
        return false;
    }
})();
const withoutAddon = installed ? 'a built addon is installed' : false;
const withAddon = installed ? false : 'no built addon is installed';

test('session construction is synchronous and side-effect free', async () => {
    const cad = new OpenCAD();
    assert.equal(cad.locale, 'en');
    assert.equal(cad.history, 'memory');
    assert.equal(cad.disposed, false);
    await cad.dispose();
    assert.equal(cad.disposed, true);
});

test('a sidecar session is declared without touching the disk', async () => {
    const cad = new OpenCAD({ history: 'sidecar' });
    assert.equal(cad.history, 'sidecar');
    assert.equal(cad.disposed, false);
    await cad.dispose();
});

test('without an installed addon engine calls keep capability/not-implemented', { skip: withoutAddon }, async () => {
    const cad = new OpenCAD({ locale: 'zh-CN' });
    assert.equal(cad.locale, 'zh-CN');
    const calls: (() => Promise<unknown>)[] = [
        () => cad.capabilities(),
        () => cad.create({ title: 'Site plan' }),
        () => cad.open({ path: 'plan.ocad' }),
        () => cad.inspect({ path: 'plan.dxf' }),
    ];
    for (const call of calls) {
        await assert.rejects(call, (error: unknown) => {
            assert.ok(error instanceof OpenCADError);
            assert.equal(error.code, 'capability/not-implemented');
            assert.ok(error.details.feature);
            return true;
        });
    }
    await cad.dispose();
});

test('dispose is idempotent and closes the session', async () => {
    const cad = new OpenCAD();
    await cad.dispose();
    await cad.dispose();
    await assert.rejects(cad.capabilities(), (error: unknown) => error instanceof OpenCADError && error.code === 'session/disposed');
});

test('a disposed session refuses create and open', async () => {
    const cad = new OpenCAD();
    await cad.dispose();
    await assert.rejects(cad.create({ title: 'Site plan' }), (error: unknown) => error instanceof OpenCADError && error.code === 'session/disposed');
    await assert.rejects(cad.open({ path: 'plan.ocad' }), (error: unknown) => error instanceof OpenCADError && error.code === 'session/disposed');
});

test('open rejects ambiguous input before reaching the engine', async () => {
    const cad = new OpenCAD();
    const inputs = [
        {} as unknown as DrawingInput,
        { path: '' } as unknown as DrawingInput,
        { path: 42 } as unknown as DrawingInput,
    ];
    for (const input of inputs) {
        await assert.rejects(
            cad.open(input),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
    }
    await cad.dispose();
});

test('format registry classifies the drawing format family', () => {
    const expected = ['dwg', 'dxf', 'step', 'stp', 'iges', 'igs', 'stl', 'obj', 'svg', 'pdf', 'png', 'jpg'];
    assert.deepEqual([...drawingFormats].sort(), [...expected].sort());

    assert.equal(formatInfo('dwg').family, 'native');
    assert.equal(formatInfo('dwg').exchangeTarget, false);
    assert.equal(formatInfo('dxf').exchangeTarget, true);
    assert.equal(formatInfo('step').family, 'exchange');
    assert.equal(formatInfo('stl').family, 'mesh');
    assert.equal(formatInfo('obj').exchangeTarget, true);
    assert.equal(formatInfo('svg').markup, 'xml');
    assert.equal(formatInfo('pdf').family, 'document');
    assert.equal(formatInfo('png').family, 'raster');

    assert.equal(formatFromPath('Site-Plan.DXF'), 'dxf');
    assert.equal(formatFromPath('plan.ocad'), undefined);
    assert.equal(formatFromPath('no-extension'), undefined);
});

test('capability matrix matches the implemented core', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    const matrix = await cad.capabilities();
    assert.equal(matrix.project, 'partial');
    assert.equal(matrix.formats.svg, 'supported');
    assert.equal(matrix.formats.pdf, 'supported');
    assert.equal(matrix.formats.dxf, 'partial');
    assert.equal(matrix.formats.dwg, 'unsupported');
    for (const check of ['units', 'layers', 'geometry', 'dimensions'] as const) {
        assert.equal(matrix.checks[check], 'supported');
    }
    for (const check of ['constraints', 'references'] as const) {
        assert.equal(matrix.checks[check], 'unsupported');
    }
    assert.equal(matrix.renderers.svg, 'supported');
    assert.equal(matrix.renderers.pdf, 'unsupported');
    assert.equal(matrix.exportTargets.svg, 'supported');
    assert.equal(matrix.exportTargets.dxf, 'supported');
    assert.equal(matrix.exportTargets.pdf, 'supported');
    assert.equal(matrix.exportTargets.stl, 'supported');
    assert.equal(matrix.exportTargets.obj, 'supported');
    assert.equal(matrix.exportTargets.step, 'supported');
    assert.equal(matrix.formats.stl, 'unsupported', 'the STL reader stays unsupported');
    assert.equal(matrix.formats.obj, 'partial', 'OBJ is export-only');
    assert.equal(matrix.formats.step, 'partial', 'STEP is export-only');
    await cad.dispose();
});

test('drawing core round trip, inspection and export', { skip: withAddon }, async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'open-cad-sdk-'));
    const file = path.join(dir, 'plan.ocad');
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Site plan', units: 'mm' });
        assert.equal(document.units, 'mm');
        assert.equal(document.layers.length, 2);
        assert.equal(document.layers[0]?.id, '0');
        assert.equal(document.layers[1]?.id, 'dimensions');

        const line = document.addLine({ layer: '0', from: [0, 0], to: [100, 40] });
        assert.equal(line.id, 'entity-1');
        assert.equal(line.kind.type, 'line');
        const circle = document.addCircle({ layer: '0', center: [50, 50], radius: 20 });
        assert.equal(circle.id, 'entity-2');
        document.addPolyline({ layer: '0', points: [[0, 0], [10, 0], [10, 10]], closed: true });
        document.addText({ layer: '0', at: [5, 5], height: 2.5, content: 'Site' });
        document.addDimension({ layer: 'dimensions', from: [0, 0], to: [100, 0], offset: 10 });
        assert.equal(document.entities, 5);
        assert.equal(document.listEntities().length, 5);

        const report = document.inspect();
        assert.equal(report.passed, true);
        assert.equal(report.issues.length, 0);
        assert.ok(report.checkerVersion.length > 0);

        const saved = document.save({ target: file, overwrite: false });
        assert.ok(saved.bytes > 0);
        assert.equal(saved.revision, document.revision);
        assert.ok(fs.existsSync(file));
        assert.match(fs.readFileSync(file, 'utf8'), /"format": "open-cad-drawing"/);

        const svgPath = path.join(dir, 'plan.svg');
        const svgReport = document.export({ format: 'svg', target: svgPath });
        assert.equal(svgReport.format, 'svg');
        assert.equal(svgReport.editable, false);
        assert.match(fs.readFileSync(svgPath, 'utf8'), /<svg/);

        const dxfPath = path.join(dir, 'plan.dxf');
        const dxfReport = document.export({ format: 'dxf', target: dxfPath });
        assert.equal(dxfReport.format, 'dxf');
        assert.ok(dxfReport.losses.length > 0);
        const dxf = fs.readFileSync(dxfPath, 'utf8');
        assert.match(dxf, /ENTITIES/);
        assert.match(dxf, /EOF/);

        const stlPath = path.join(dir, 'plan.stl');
        const stlReport = document.export({ format: 'stl', target: stlPath });
        assert.equal(stlReport.format, 'stl');
        assert.ok((stlReport.triangles ?? 0) > 0);
        const stl = fs.readFileSync(stlPath, 'utf8');
        assert.ok(stl.startsWith('solid open-cad'));
        assert.match(stl, /facet normal/);
        assert.ok(stl.trimEnd().endsWith('endsolid'));

        const objPath = path.join(dir, 'plan.obj');
        const objReport = document.export({ format: 'obj', target: objPath });
        assert.equal(objReport.format, 'obj');
        assert.equal(objReport.editable, false);
        assert.ok((objReport.triangles ?? 0) > 0);
        const obj = fs.readFileSync(objPath, 'utf8');
        assert.ok(obj.startsWith('# OpenCAD OBJ extrusion'));
        assert.match(obj, /^v \S+ \S+ \S+$/m);
        assert.match(obj, /^f \d+ \d+ \d+$/m);

        const pdfPath = path.join(dir, 'plan.pdf');
        const pdfReport = document.export({ format: 'pdf', target: pdfPath });
        assert.equal(pdfReport.format, 'pdf');
        assert.equal(pdfReport.editable, false);
        const pdf = fs.readFileSync(pdfPath, 'utf8');
        assert.match(pdf, /^%PDF-1\.[4-7]/);
        assert.match(pdf, /xref/);
        assert.match(pdf, /trailer/);
        assert.ok(pdf.trimEnd().endsWith('%%EOF'));

        const stepPath = path.join(dir, 'plan.step');
        const stepReport = document.export({ format: 'step', target: stepPath });
        assert.equal(stepReport.format, 'step');
        assert.equal(stepReport.editable, false);
        assert.ok((stepReport.faces ?? 0) > 0);
        const step = fs.readFileSync(stepPath, 'utf8');
        assert.ok(step.startsWith('ISO-10303-21;'));
        assert.match(step, /MANIFOLD_SOLID_BREP/);
        assert.ok(step.trimEnd().endsWith('END-ISO-10303-21;'));

        const deletion = document.deleteEntity('entity-1');
        assert.equal(deletion.deleted, 'entity-1');
        assert.equal(document.entities, 4);
        assert.equal(deletion.drawing.entities, 4);

        document.close();
        document.close();
        assert.equal(document.isClosed, true);
        assert.throws(() => document.listEntities(), (error: unknown) => error instanceof OpenCADError);

        const second = new OpenCAD();
        const reopened = await second.open({ path: file });
        assert.equal(reopened.entities, 5);
        assert.equal(reopened.revision, saved.revision);
        await second.dispose();
    } finally {
        await cad.dispose();
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test('editing refuses an unknown layer', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({});
        assert.throws(
            () => document.addLine({ layer: 'missing', from: [0, 0], to: [1, 1] }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/not-found',
        );
    } finally {
        await cad.dispose();
    }
});

test('layer merge moves references and removes the source layer', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Merge' });
        document.addLayer({ name: 'Walls' });
        document.addLayer({ name: 'Structure' });
        document.addLine({ layer: 'Walls', from: [0, 0], to: [10, 0] });

        const merged = document.mergeLayer({ from: 'Walls', to: 'Structure' });
        assert.equal(merged.from, 'Walls');
        assert.equal(merged.to, 'Structure');
        assert.equal(merged.moved, 1);
        assert.equal(document.layers.some((layer) => layer.id === 'Walls'), false);
        assert.equal(document.layers.find((layer) => layer.id === 'Structure')?.entityCount, 1);
        const line = document.listEntities()[0];
        assert.equal(line?.layer, 'Structure');
        assert.throws(
            () => document.mergeLayer({ from: 'Walls', to: 'Structure' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/not-found',
        );
        document.undo();
        assert.equal(document.layers.some((layer) => layer.id === 'Walls'), true);
    } finally {
        await cad.dispose();
    }
});

test('dimension styles are read and written through the document', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Styles' });
        assert.equal(document.dimStyles.length, 1);
        assert.equal(document.dimStyles[0]?.name, 'standard');

        const added = document.addDimStyle({ name: 'ARCH', scale: 2.5, textHeight: 3.5, precision: 0 });
        assert.equal(added.id, 'ARCH');
        assert.equal(added.scale, 2.5);
        assert.equal(added.arrowSize, 2.5);
        assert.equal(added.precision, 0);

        const updated = document.updateDimStyle({ styleId: 'ARCH', name: 'ARCH-1', arrowSize: 4 });
        assert.equal(updated.id, 'ARCH-1');
        assert.equal(updated.arrowSize, 4);
        assert.equal(document.dimStyles.length, 2);

        const deleted = document.deleteDimStyle('ARCH-1');
        assert.equal(deleted.deleted, 'ARCH-1');
        assert.equal(document.dimStyles.length, 1);
        assert.throws(
            () => document.deleteDimStyle('standard'),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/style-last',
        );
    } finally {
        await cad.dispose();
    }
});

test('text styles are read and written through the document', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Styles' });
        assert.equal(document.textStyles.length, 1);
        assert.equal(document.textStyles[0]?.font, 'txt');

        const added = document.addTextStyle({ name: 'NOTES', font: 'romans.shx', height: 2.5, widthFactor: 0.8, oblique: 15 });
        assert.equal(added.id, 'NOTES');
        assert.equal(added.font, 'romans.shx');
        assert.equal(added.widthFactor, 0.8);

        const updated = document.updateTextStyle({ styleId: 'NOTES', name: 'NOTES-1', oblique: 0 });
        assert.equal(updated.id, 'NOTES-1');
        assert.equal(updated.oblique, 0);
        assert.equal(document.textStyles.length, 2);

        const deleted = document.deleteTextStyle('NOTES-1');
        assert.equal(deleted.deleted, 'NOTES-1');
        assert.equal(document.textStyles.length, 1);
        assert.throws(
            () => document.deleteTextStyle('standard'),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/style-last',
        );
    } finally {
        await cad.dispose();
    }
});

test('linetypes and layer lineweights are read and written', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Line types' });
        assert.equal(document.linetypes.length, 1);
        assert.equal(document.linetypes[0]?.name, 'continuous');

        const dashed = document.addLinetype({ name: 'DASHED', description: 'Dashed', pattern: [5, 2] });
        assert.equal(dashed.id, 'DASHED');
        assert.deepEqual(dashed.pattern, [5, 2]);

        const layer = document.updateLayer({ layerId: '0', linetype: 'DASHED', lineweight: 0.5 });
        assert.equal(layer.linetype, 'DASHED');
        assert.equal(layer.lineweight, 0.5);

        const renamed = document.updateLinetype({ linetypeId: 'DASHED', name: 'DASH-1' });
        assert.equal(renamed.id, 'DASH-1');
        assert.equal(document.layers[0]?.linetype, 'DASH-1');

        assert.throws(
            () => document.deleteLinetype('continuous'),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/linetype-in-use',
        );
        assert.throws(
            () => document.updateLayer({ layerId: '0', linetype: 'missing' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/not-found',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('layer freeze and plot flags gate writes', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Layer flags' });
        const layer = document.updateLayer({ layerId: '0', frozen: true, plottable: false });
        assert.equal(layer.frozen, true);
        assert.equal(layer.plottable, false);
        assert.throws(
            () => document.addLine({ layer: '0', from: [0, 0], to: [1, 1] }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/layer-frozen',
        );
        const cleared = document.updateLayer({ layerId: '0', frozen: false, plottable: true });
        assert.equal(cleared.frozen, false);
        assert.equal(cleared.plottable, true);
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('selection returns window and crossing matches', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Selection' });
        document.addLayer({ name: 'Walls' });
        const first = document.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        const second = document.addLine({ layer: 'Walls', from: [10, 10], to: [20, 20] });

        const window = document.select({ window: [[-1, -1], [11, 1]] });
        assert.equal(window.mode, 'window');
        assert.deepEqual(window.ids, [first.id]);

        const crossing = document.select({ window: [[9, -1], [11, 11]], mode: 'crossing' });
        assert.deepEqual([...crossing.ids].sort(), [first.id, second.id].sort());

        const walls = document.select({ window: [[9, -1], [11, 11]], mode: 'crossing', layer: 'Walls' });
        assert.deepEqual(walls.ids, [second.id]);
        assert.equal(walls.layer, 'Walls');
    } finally {
        await cad.dispose();
    }
});

test('snap reports endpoints, midpoints, centres and intersections', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Snap' });
        const horizontal = document.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        document.addLine({ layer: '0', from: [5, -5], to: [5, 5] });
        document.addCircle({ layer: '0', center: [20, 20], radius: 2 });

        const all = document.snap();
        assert.equal(all.count, 8);
        const intersections = document.snap({ modes: ['intersection'] });
        assert.equal(intersections.count, 1);
        assert.deepEqual(intersections.points[0]?.point, [5, 0]);

        const near = document.snap({ modes: ['endpoint'], point: [9.9, 0], tolerance: 0.5 });
        assert.equal(near.count, 1);
        assert.deepEqual(near.points[0]?.entities, [horizontal.id]);
        assert.equal(near.points[0]?.kind, 'endpoint');

        assert.throws(
            () => document.snap({ modes: ['edge'] as unknown as ('endpoint' | 'midpoint' | 'center' | 'intersection')[] }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('snap covers quadrant, nearest and curved intersections', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Snap modes' });
        const circle = document.addCircle({ layer: '0', center: [0, 0], radius: 2 });
        const arc = document.addArc({ layer: '0', center: [100, 0], radius: 10, startAngle: 0, endAngle: 90 });
        const line = document.addLine({ layer: '0', from: [-5, 0], to: [5, 0] });
        const outer = document.addCircle({ layer: '0', center: [0, 0], radius: 5 });
        const second = document.addCircle({ layer: '0', center: [6, 0], radius: 5 });
        const hasPoint = (result: { points: readonly { point: readonly [number, number] }[] }, expected: readonly [number, number]) =>
            result.points.some((entry) => Math.abs(entry.point[0] - expected[0]) < 1e-9 && Math.abs(entry.point[1] - expected[1]) < 1e-9);

        const quadrants = document.snap({ modes: ['quadrant'], ids: [circle.id, arc.id] });
        assert.equal(quadrants.count, 6);
        assert.ok(hasPoint(quadrants, [100, 10]));
        assert.ok(!hasPoint(quadrants, [100, -10]), 'an arc must not report a quadrant outside its sweep');

        const nearest = document.snap({ modes: ['nearest'], point: [20, 0], ids: [circle.id] });
        assert.equal(nearest.count, 1);
        assert.equal(nearest.points[0]?.kind, 'nearest');
        assert.ok(hasPoint(nearest, [2, 0]));

        const lineCircle = document.snap({ modes: ['intersection'], ids: [line.id, circle.id] });
        assert.equal(lineCircle.count, 2);
        assert.ok(hasPoint(lineCircle, [-2, 0]));
        assert.ok(hasPoint(lineCircle, [2, 0]));

        const circleCircle = document.snap({ modes: ['intersection'], ids: [outer.id, second.id] });
        assert.equal(circleCircle.count, 2);
        assert.ok(hasPoint(circleCircle, [3, 4]));
        assert.ok(hasPoint(circleCircle, [3, -4]));

        assert.throws(
            () => document.snap({ modes: ['nearest'] }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('unit conversion scales lengths and round trips', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Units', units: 'mm' });
        document.addLine({ layer: '0', from: [0, 0], to: [25.4, 0] });
        const revision = document.revision;

        const inches = document.convertUnits({ to: 'inch' });
        assert.equal(inches.from, 'mm');
        assert.equal(inches.to, 'inch');
        assert.equal(inches.converted, true);
        assert.equal(document.units, 'inch');
        assert.equal(inches.revision, revision + 1);
        const line = document.listEntities()[0];
        if (line?.kind.type === 'line') {
            assert.ok(Math.abs(line.kind.to[0] - 1) < 1e-12);
        }

        const same = document.convertUnits({ to: 'inch' });
        assert.equal(same.converted, false);
        assert.equal(same.revision, inches.revision);

        const millimetres = document.convertUnits({ to: 'mm' });
        assert.equal(millimetres.converted, true);
        const back = document.listEntities()[0];
        if (back?.kind.type === 'line') {
            assert.ok(Math.abs(back.kind.to[0] - 25.4) < 1e-9);
        }
    } finally {
        await cad.dispose();
    }
});

test('radial and ordinate dimensions are editable entities', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Dimensions' });
        const radial = document.addRadialDimension({ layer: 'dimensions', center: [0, 0], radius: 10, angle: 0, offset: 2 });
        assert.equal(radial.kind.type, 'radial_dimension');
        const diameter = document.addRadialDimension({ layer: 'dimensions', center: [0, 0], radius: 10, angle: 90, diameter: true });
        assert.equal(diameter.kind.type, 'radial_dimension');
        const ordinate = document.addOrdinateDimension({
            layer: 'dimensions',
            origin: [0, 0],
            feature: [10, 5],
            axis: 'x',
            offset: 2,
        });
        assert.equal(ordinate.kind.type, 'ordinate_dimension');
        assert.equal(document.entities, 3);
        assert.equal(document.inspect({ checks: ['dimensions'] }).issues.length, 0);
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('find and replace update every matching text entity', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Text search' });
        const first = document.addText({ layer: '0', at: [0, 0], height: 2.5, content: 'Main Hall' });
        const second = document.addText({ layer: '0', at: [0, 5], height: 2.5, content: 'Side hall' });
        document.addText({ layer: '0', at: [0, 10], height: 2.5, content: 'Room' });

        const found = document.findText({ pattern: 'hall' });
        assert.equal(found.count, 2);
        assert.deepEqual(found.matches.map((match) => match.id), [first.id, second.id]);

        const revision = document.revision;
        const replaced = document.replaceText({ pattern: 'hall', replacement: 'lobby' });
        assert.equal(replaced.count, 2);
        assert.equal(replaced.occurrences, 2);
        assert.equal(replaced.revision, revision + 1);
        const contents = document.listEntities().flatMap((entity) => (entity.kind.type === 'text' ? [entity.kind.content] : []));
        assert.deepEqual(contents, ['Main lobby', 'Side lobby', 'Room']);
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('a hatch keeps its closed boundary and pattern name', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Hatch' });
        document.addLayer({ name: 'fills' });
        const solid = document.addHatch({ layer: 'fills', points: [[0, 0], [10, 0], [10, 10], [0, 10]] });
        assert.equal(solid.kind.type, 'hatch');
        const patterned = document.addHatch({ layer: 'fills', points: [[20, 0], [30, 0], [25, 8]], pattern: 'ANSI31' });
        assert.equal(patterned.kind.type, 'hatch');
        assert.deepEqual(document.measure().area, 140);
        assert.equal(document.stats().totals.entities, 2);
        assert.equal(document.inspect({ checks: ['geometry'] }).issues.length, 0);
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('rectangles and regular polygons become closed polylines', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Shapes' });
        const rectangle = document.addRectangle({ layer: '0', from: [0, 0], to: [40, 20] });
        assert.equal(rectangle.kind.type, 'polyline');
        if (rectangle.kind.type === 'polyline') {
            assert.equal(rectangle.kind.closed, true);
            assert.deepEqual(rectangle.kind.points, [[0, 0], [40, 0], [40, 20], [0, 20]]);
        }

        const hexagon = document.addPolygon({ layer: '0', center: [100, 50], radius: 10, sides: 6, rotation: 0 });
        assert.equal(hexagon.kind.type, 'polyline');
        if (hexagon.kind.type === 'polyline') {
            assert.equal(hexagon.kind.closed, true);
            assert.equal(hexagon.kind.points.length, 6);
            assert.ok(Math.abs(hexagon.kind.points[0][0] - 110) < 1e-9);
            assert.ok(Math.abs(hexagon.kind.points[0][1] - 50) < 1e-9);
        }

        const listed = document.listEntities();
        assert.equal(listed.length, 2);
        assert.deepEqual(listed.map((entity) => entity.kind.type), ['polyline', 'polyline']);

        const measured = document.measureEntities([rectangle.id, hexagon.id]);
        assert.deepEqual(measured.entities.map((entity) => entity.closed), [true, true]);
        assert.ok(Math.abs(measured.entities[0].area - 800) < 1e-9);
        assert.ok(Math.abs(measured.entities[1].area - (3 * Math.sqrt(3) / 2) * 100) < 1e-9);

        assert.throws(
            () => document.addRectangle({ layer: '0', from: [10, 10], to: [0, 0] }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        assert.throws(
            () => document.addPolygon({ layer: '0', center: [0, 0], radius: 5, sides: 2 }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('offset copies circle and polyline geometry as new entities', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Offset' });
        const circle = document.addCircle({ layer: '0', center: [0, 0], radius: 10 });
        const grown = document.offset({ entityId: circle.id, distance: 2 });
        assert.equal(grown.source, circle.id);
        assert.equal(grown.entity.kind.type, 'circle');
        if (grown.entity.kind.type === 'circle') {
            assert.equal(grown.entity.kind.radius, 12);
        }
        assert.equal(document.entities, 2);

        const polyline = document.addPolyline({ layer: '0', points: [[0, 0], [10, 0], [10, 10]], closed: false });
        const moved = document.offset({ entityId: polyline.id, distance: 1 });
        assert.equal(moved.entity.kind.type, 'polyline');
        if (moved.entity.kind.type === 'polyline') {
            assert.deepEqual(moved.entity.kind.points, [[0, 1], [9, 1], [9, 10]]);
        }
        assert.equal(document.entities, 4);
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('offsetBySide creates a new entity with side-aware geometry', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Offset by side' });
        const circle = document.addCircle({ layer: '0', center: [0, 0], radius: 10 });
        const grown = document.offsetBySide({ entityId: circle.id, distance: 2, side: 'outside' });
        assert.equal(grown.source, circle.id);
        assert.equal(grown.side, 'outside');
        assert.equal(grown.distance, 2);
        assert.equal(grown.entity.kind.type, 'circle');
        assert.notEqual(grown.entity.id, circle.id);
        if (grown.entity.kind.type === 'circle') {
            assert.equal(grown.entity.kind.radius, 12);
        }

        const shrunk = document.offsetBySide({ entityId: circle.id, distance: 4, side: 'inside' });
        if (shrunk.entity.kind.type === 'circle') {
            assert.equal(shrunk.entity.kind.radius, 6);
        }

        const line = document.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        const left = document.offsetBySide({ entityId: line.id, distance: 2, side: 'left' });
        assert.deepEqual(left.entity.kind, { type: 'line', from: [0, 2], to: [10, 2] });
        const right = document.offsetBySide({ entityId: line.id, distance: 3, side: 'right' });
        assert.deepEqual(right.entity.kind, { type: 'line', from: [0, -3], to: [10, -3] });
        const signed = document.offset({ entityId: line.id, distance: 2 });
        assert.deepEqual(signed.entity.kind, left.entity.kind, 'signed and side offsets agree');
        assert.deepEqual(document.listEntities().find((entity) => entity.id === line.id)?.kind, {
            type: 'line',
            from: [0, 0],
            to: [10, 0],
        });

        const polyline = document.addPolyline({ layer: '0', points: [[0, 0], [10, 0], [10, 10]], closed: false });
        const moved = document.offsetBySide({ entityId: polyline.id, distance: 1, side: 'left' });
        if (moved.entity.kind.type === 'polyline') {
            assert.deepEqual(moved.entity.kind.points, [[0, 1], [9, 1], [9, 10]]);
        }

        await assert.rejects(
            async () => document.offsetBySide({ entityId: circle.id, distance: 20, side: 'inside' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'offset/invalid-distance',
        );
        await assert.rejects(
            async () => document.offsetBySide({ entityId: circle.id, distance: -1, side: 'outside' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'offset/invalid-distance',
        );
        await assert.rejects(
            async () => document.offsetBySide({ entityId: circle.id, distance: 1, side: 'left' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'offset/invalid-side',
        );

        const sharp = document.addPolyline({ layer: '0', points: [[0, 0], [10, 0], [0, 0.1]], closed: false });
        await assert.rejects(
            async () => document.offsetBySide({ entityId: sharp.id, distance: 1, side: 'left' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'offset/miter-limit',
        );

        const text = document.addText({ layer: '0', at: [0, 0], height: 2, content: 'note' });
        await assert.rejects(
            async () => document.offsetBySide({ entityId: text.id, distance: 1, side: 'left' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'offset/unsupported-entity',
        );
        await assert.rejects(
            async () => document.offsetBySide({ entityId: 'entity-404', distance: 1, side: 'left' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'offset/not-found',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('fillet and chamfer trim two lines and append the corner entity', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Corners' });
        const horizontal = document.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        const vertical = document.addLine({ layer: '0', from: [10, 0], to: [10, 10] });
        const arc = document.fillet(horizontal.id, vertical.id, 2);
        assert.equal(arc.kind.type, 'arc');
        assert.deepEqual(document.listEntities().find((entity) => entity.id === horizontal.id)?.kind, {
            type: 'line',
            from: [0, 0],
            to: [8, 0],
        });

        const document2 = await cad.create({ title: 'Chamfers' });
        const a = document2.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        const b = document2.addLine({ layer: '0', from: [10, 0], to: [10, 10] });
        const bevel = document2.chamfer(a.id, b.id, 3, 4);
        assert.equal(bevel.kind.type, 'line');
        if (bevel.kind.type === 'line') {
            assert.deepEqual(bevel.kind.from, [7, 0]);
            assert.deepEqual(bevel.kind.to, [10, 4]);
        }
        document.close();
        document2.close();
    } finally {
        await cad.dispose();
    }
});

test('trim cuts a line at a cutter and keeps the longer side', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Trim' });
        const target = document.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        const cutter = document.addCircle({ layer: '0', center: [7, 0], radius: 2 });
        const trimmed = document.trim(target.id, cutter.id);
        assert.equal(trimmed.cutter, cutter.id);
        assert.equal(trimmed.entity.id, target.id);
        assert.deepEqual(trimmed.entity.kind, { type: 'line', from: [0, 0], to: [5, 0] });

        const text = document.addText({ layer: '0', at: [0, 0], height: 2, content: 'note' });
        assert.throws(
            () => document.trim(target.id, text.id),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/unsupported',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('extend lengthens a line to its nearest boundary', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Extend' });
        const target = document.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        const boundary = document.addCircle({ layer: '0', center: [16, 0], radius: 2 });
        const extended = document.extend(target.id, boundary.id);
        assert.equal(extended.boundary, boundary.id);
        assert.equal(extended.side, 'end');
        assert.equal(extended.entity.id, target.id);
        assert.deepEqual(extended.entity.kind, { type: 'line', from: [0, 0], to: [14, 0] });

        const text = document.addText({ layer: '0', at: [0, 0], height: 2, content: 'note' });
        assert.throws(
            () => document.extend(target.id, text.id),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/unsupported',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('align and distribute move entities by bounding boxes', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Align' });
        const first = document.addLine({ layer: '0', from: [0, 0], to: [2, 0] });
        const second = document.addLine({ layer: '0', from: [3, 0], to: [9, 0] });
        const third = document.addLine({ layer: '0', from: [20, 0], to: [24, 0] });

        const aligned = document.align({ ids: [first.id, second.id], mode: 'left' });
        assert.equal(aligned.mode, 'left');
        assert.equal(aligned.affected, 2);
        assert.equal(aligned.bounds.minX, 0);

        const distributed = document.distribute({ ids: [first.id, second.id, third.id], axis: 'x' });
        assert.equal(distributed.axis, 'x');
        assert.equal(distributed.affected, 3);
        assert.equal(distributed.bounds.maxX, 24);
        const [movedFirst, movedSecond, movedThird] = document.listEntities();
        assert.deepEqual(movedFirst?.kind, { type: 'line', from: [0, 0], to: [2, 0] });
        assert.deepEqual(movedSecond?.kind, { type: 'line', from: [8, 0], to: [14, 0] });
        assert.deepEqual(movedThird?.kind, { type: 'line', from: [20, 0], to: [24, 0] });

        assert.throws(
            () => document.align({ ids: [first.id], mode: 'left' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'align/insufficient-entities',
        );
        assert.throws(
            () => document.distribute({ ids: [first.id, second.id], axis: 'x' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'distribute/insufficient-entities',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('entity measurement reports area and perimeter', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Measure entities' });
        const square = document.addPolyline({ layer: '0', points: [[0, 0], [10, 0], [10, 10], [0, 10]], closed: true });
        const line = document.addLine({ layer: '0', from: [0, 0], to: [3, 4] });

        const measured = document.measureEntities([square.id, line.id]);
        assert.equal(measured.totals.entities, 2);
        assert.equal(measured.totals.area, 100);
        assert.equal(measured.totals.length, 45);
        assert.deepEqual(measured.entities[0], {
            id: square.id,
            layer: '0',
            kind: 'polyline',
            closed: true,
            length: 40,
            area: 100,
        });
        assert.equal(measured.entities[1]?.area, 0);
        assert.throws(
            () => document.measureEntities(['entity-9']),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/not-found',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('entity groups keep membership without changing geometry', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Groups' });
        const first = document.addLine({ layer: '0', from: [0, 0], to: [1, 0] });
        const second = document.addLine({ layer: '0', from: [0, 1], to: [1, 1] });

        const created = document.createGroup({ name: 'Frame', entities: [first.id, second.id] });
        assert.deepEqual(created.entities, [first.id, second.id]);
        assert.equal(document.summary().groups, 1);
        assert.equal(document.listGroups()[0]?.name, 'Frame');

        const added = document.addGroupMembers({ groupId: 'Frame', entities: [first.id] });
        assert.equal(added.entities.length, 2);
        document.deleteEntity(first.id);
        assert.deepEqual(document.listGroups()[0]?.entities, [second.id]);

        assert.throws(
            () => document.createGroup({ name: 'frame', entities: [second.id] }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/group-exists',
        );
        document.deleteGroup('Frame');
        assert.equal(document.listGroups().length, 0);
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('drawing properties carry the title block', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Site plan' });
        assert.equal(document.title, 'Site plan');
        assert.equal(document.properties.author, '');

        const updated = document.updateProperties({ author: ' Design team ', company: 'OpenCAD', project: 'Harbour' });
        assert.equal(updated.author, 'Design team');
        assert.equal(updated.company, 'OpenCAD');
        assert.equal(document.properties.project, 'Harbour');
        assert.equal(document.summary().properties.company, 'OpenCAD');

        document.updateProperties({ title: 'Harbour plan' });
        assert.equal(document.title, 'Harbour plan');
        assert.throws(
            () => document.updateProperties({}),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('measurePoints reports a distance or an angle', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Point measurement' });
        const distance = document.measurePoints([[0, 0], [3, 4]]);
        assert.equal(distance.kind, 'distance');
        if (distance.kind === 'distance') {
            assert.equal(distance.distance, 5);
            assert.deepEqual([distance.dx, distance.dy], [3, 4]);
        }
        const angle = document.measurePoints([[0, 10], [0, 0], [5, 0]]);
        assert.equal(angle.kind, 'angle');
        if (angle.kind === 'angle') {
            assert.equal(angle.angle, 90);
            assert.equal(angle.first, 10);
            assert.equal(angle.second, 5);
        }
        assert.throws(
            () => document.measurePoints([[0, 0]]),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('ucs converts points and rotates entities', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'UCS' });
        const line = document.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        const converted = document.ucs({ origin: [10, 0], rotation: 90, direction: 'to-ucs', points: [[10, 10]] });
        assert.equal(converted.direction, 'to-ucs');
        assert.ok(Math.abs((converted.points[0]?.to[0] ?? 0) - 10) < 1e-9);
        assert.ok(Math.abs(converted.points[0]?.to[1] ?? 1) < 1e-9);

        const moved = document.ucs({ origin: [10, 0], rotation: 90, direction: 'to-world', ids: [line.id] });
        assert.deepEqual(moved.entities, [line.id]);
        const updated = document.listEntities()[0];
        if (updated?.kind.type === 'line') {
            assert.ok(Math.abs(updated.kind.from[0] - 10) < 1e-9);
            assert.ok(Math.abs(updated.kind.to[1] - 10) < 1e-9);
        }
        assert.throws(
            () => document.ucs({}),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('stats and the empty-layer check summarise layers', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Stats' });
        document.addLine({ layer: '0', from: [0, 0], to: [3, 4] });
        const stats = document.stats();
        assert.equal(stats.totals.entities, 1);
        assert.equal(stats.totals.length, 5);
        const defaultLayer = stats.layers.find((layer) => layer.id === '0');
        assert.equal(defaultLayer?.entityCount, 1);
        const dimensions = stats.layers.find((layer) => layer.id === 'dimensions');
        assert.equal(dimensions?.entityCount, 0);

        const report = document.inspect({ checks: ['empty-layer'] });
        assert.equal(report.passed, true);
        const empty = report.issues.filter((issue) => issue.code === 'layers/empty');
        assert.equal(empty.length, 1);
        assert.equal(empty[0]?.layerId, 'dimensions');
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('stats grouping and the quantity table draw aggregates', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Take-off' });
        document.addLine({ layer: '0', from: [0, 0], to: [3, 0] });
        document.addCircle({ layer: '0', center: [0, 0], radius: 1 });
        const byType = document.stats('type');
        assert.equal(byType.groupBy, 'type');
        assert.deepEqual(byType.groups.map((group) => group.key), ['circle', 'line']);
        const table = document.quantityTable({ origin: [10, 10], title: 'Bill' });
        assert.equal(table.groupBy, 'layer');
        assert.equal(table.totals.entities, 2);
        assert.ok(table.created.length > 0);
        assert.ok(table.created.every((entity) => entity.layer === 'quantity'));
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('polyline vertices and the closed flag are editable', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Polyline edits' });
        const polyline = document.addPolyline({ layer: '0', points: [[0, 0], [10, 0], [10, 10]] });

        const closed = document.updatePolyline({ entityId: polyline.id, closed: true });
        assert.equal(closed.kind.type, 'polyline');
        if (closed.kind.type === 'polyline') {
            assert.equal(closed.kind.closed, true);
        }

        const inserted = document.updatePolyline({ entityId: polyline.id, insertAt: 1, insertPoint: [5, 0] });
        if (inserted.kind.type === 'polyline') {
            assert.deepEqual(inserted.kind.points, [[0, 0], [5, 0], [10, 0], [10, 10]]);
        }
        const removed = document.updatePolyline({ entityId: polyline.id, removeAt: 1 });
        if (removed.kind.type === 'polyline') {
            assert.deepEqual(removed.kind.points, [[0, 0], [10, 0], [10, 10]]);
        }
        const moved = document.updatePolyline({ entityId: polyline.id, moveAt: 1, moveTo: [12, 3] });
        if (moved.kind.type === 'polyline') {
            assert.deepEqual(moved.kind.points, [[0, 0], [12, 3], [10, 10]]);
        }

        const replaced = document.updatePolyline({
            entityId: polyline.id,
            points: [[0, 0], [4, 0], [4, 4], [0, 4]],
            closed: true,
        });
        if (replaced.kind.type === 'polyline') {
            assert.deepEqual(replaced.kind.points, [[0, 0], [4, 0], [4, 4], [0, 4]]);
            assert.equal(replaced.kind.closed, true);
        }
        assert.equal(document.measureEntities([polyline.id]).entities[0]?.area, 16);

        assert.throws(
            () => document.updatePolyline({ entityId: polyline.id, removeAt: 9 }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('polyline smoothing rounds corners and keeps the entity', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Polyline smoothing' });
        const polyline = document.addPolyline({ layer: '0', points: [[0, 0], [10, 0], [10, 10]] });

        const smoothed = document.smoothPolyline({ id: polyline.id, iterations: 1 });
        assert.equal(smoothed.id, polyline.id);
        assert.equal(smoothed.layer, '0');
        if (smoothed.kind.type === 'polyline') {
            assert.deepEqual(smoothed.kind.points, [[0, 0], [2.5, 0], [7.5, 0], [10, 2.5], [10, 7.5], [10, 10]]);
            assert.equal(smoothed.kind.closed, false);
        }
        assert.equal(document.revision, 3);

        assert.throws(
            () => document.smoothPolyline({ id: polyline.id, iterations: 0 }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'polyline/invalid-iterations',
        );
        assert.throws(
            () => document.smoothPolyline({ id: polyline.id, ratio: 0.5 }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'polyline/invalid-ratio',
        );
        assert.throws(
            () => document.smoothPolyline({ id: 'entity-99' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/not-found',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('diff reports entity, layer and metadata differences between two projects', { skip: withAddon }, async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'open-cad-diff-'));
    const left = path.join(dir, 'left.ocad');
    const right = path.join(dir, 'right.ocad');
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Left plan' });
        document.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        const circle = document.addCircle({ layer: '0', center: [5, 5], radius: 2 });
        document.save({ target: left, overwrite: false });

        document.deleteEntity('entity-1');
        document.moveEntity({ entityId: circle.id, dx: 1, dy: 0 });
        document.addLine({ layer: '0', from: [0, 20], to: [10, 20] });
        document.updateLayer({ layerId: '0', color: '#ff0000' });
        document.updateProperties({ title: 'Right plan' });
        document.save({ target: right, overwrite: false });

        const report = await cad.diff({ left, right });
        assert.equal(report.identical, false);
        assert.equal(report.summary.added, 1);
        assert.equal(report.summary.removed, 1);
        assert.equal(report.summary.changed, 1);
        assert.equal(report.summary.layersChanged, 1);
        assert.equal(report.summary.metadata, 1);
        assert.equal(report.entities.removed[0]?.id, 'entity-1');
        assert.deepEqual(report.entities.changed[0]?.fields, ['geometry']);
        assert.deepEqual(report.layers.changed[0]?.fields, ['color']);
        assert.equal(report.metadata[0]?.field, 'title');

        const same = await cad.diff({ left, right: left });
        assert.equal(same.identical, true);
        assert.equal(same.summary.added, 0);

        await assert.rejects(
            cad.diff({ left: path.join(dir, 'missing.ocad'), right }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/not-found',
        );
        await assert.rejects(
            cad.diff({ left: '', right } as unknown as { left: string; right: string }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('arc radius and sweep angles are editable', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Arc edits' });
        const arc = document.addArc({ layer: '0', center: [0, 0], radius: 5, startAngle: 0, endAngle: 90 });
        const updated = document.updateArc({ entityId: arc.id, radius: 8, startAngle: 45, endAngle: 180 });
        if (updated.kind.type === 'arc') {
            assert.equal(updated.kind.radius, 8);
            assert.equal(updated.kind.start_angle, 45);
            assert.equal(updated.kind.end_angle, 180);
        }
        const full = document.updateArc({ entityId: arc.id, endAngle: 45 });
        if (full.kind.type === 'arc') {
            assert.equal(full.kind.start_angle, 45);
            assert.equal(full.kind.end_angle, 45);
        }

        const line = document.addLine({ layer: '0', from: [0, 0], to: [1, 0] });
        assert.throws(
            () => document.updateArc({ entityId: line.id, radius: 2 }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/unsupported',
        );
        assert.throws(
            () => document.updateArc({ entityId: arc.id }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('ellipse and spline editing keep identity and enforce limits', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Curve edits' });
        const ellipse = document.addEllipse({ layer: '0', center: [0, 0], rx: 10, ry: 5, rotation: 0 });
        const updatedEllipse = document.updateEllipse({ entityId: ellipse.id, rx: 12, rotation: 15 });
        assert.equal(updatedEllipse.id, ellipse.id);
        assert.equal(updatedEllipse.layer, '0');
        if (updatedEllipse.kind.type === 'ellipse') {
            assert.equal(updatedEllipse.kind.rx, 12);
            assert.equal(updatedEllipse.kind.ry, 5);
            assert.equal(updatedEllipse.kind.rotation, 15);
            assert.equal(updatedEllipse.kind.start_angle, 0);
        }
        assert.throws(
            () => document.updateEllipse({ entityId: ellipse.id, rx: 0 }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );
        assert.throws(
            () => document.updateEllipse({ entityId: ellipse.id }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );

        const spline = document.addSpline({ layer: '0', points: [[0, 0], [10, 5], [20, 0]] });
        const closed = document.updateSpline({ entityId: spline.id, closed: true });
        if (closed.kind.type === 'spline') {
            assert.equal(closed.kind.closed, true);
        }
        const inserted = document.updateSpline({ entityId: spline.id, insertAt: 1, insertPoint: [5, 6] });
        if (inserted.kind.type === 'spline') {
            assert.deepEqual(inserted.kind.points, [[0, 0], [5, 6], [10, 5], [20, 0]]);
        }
        const moved = document.updateSpline({ entityId: spline.id, moveAt: 1, moveTo: [6, 7] });
        if (moved.kind.type === 'spline') {
            assert.deepEqual(moved.kind.points, [[0, 0], [6, 7], [10, 5], [20, 0]]);
        }
        const removed = document.updateSpline({ entityId: spline.id, removeAt: 1 });
        if (removed.kind.type === 'spline') {
            assert.deepEqual(removed.kind.points, [[0, 0], [10, 5], [20, 0]]);
        }
        assert.throws(
            () => document.updateSpline({ entityId: spline.id, points: [[0, 0]] }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'session/argument-invalid',
        );

        const line = document.addLine({ layer: '0', from: [0, 0], to: [1, 0] });
        assert.throws(
            () => document.updateEllipse({ entityId: line.id, rx: 5 }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/unsupported',
        );
        assert.throws(
            () => document.updateSpline({ entityId: line.id, closed: true }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/unsupported',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

test('an entity can move to another layer without changing geometry', { skip: withAddon }, async () => {
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Layer assignment' });
        document.addLayer({ name: 'Walls' });
        const line = document.addLine({ layer: '0', from: [0, 0], to: [10, 0] });
        const revision = document.revision;

        const moved = document.assignEntityLayer({ entityId: line.id, layer: 'Walls' });
        assert.equal(moved.from, '0');
        assert.equal(moved.to, 'Walls');
        assert.equal(moved.moved, true);
        assert.equal(moved.entity.layer, 'Walls');
        assert.deepEqual(moved.entity.kind, line.kind);
        assert.equal(moved.revision, revision + 1);

        const same = document.assignEntityLayer({ entityId: line.id, layer: 'Walls' });
        assert.equal(same.moved, false);
        assert.equal(same.revision, moved.revision);

        assert.throws(
            () => document.assignEntityLayer({ entityId: line.id, layer: 'missing' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'object/not-found',
        );
        document.updateLayer({ layerId: 'Walls', locked: true });
        assert.throws(
            () => document.assignEntityLayer({ entityId: line.id, layer: 'Walls' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/layer-locked',
        );
        document.close();
    } finally {
        await cad.dispose();
    }
});

const sampleDxf = [
    '0', 'SECTION', '2', 'HEADER', '9', '$INSUNITS', '70', '4', '0', 'ENDSEC',
    '0', 'SECTION', '2', 'TABLES', '0', 'TABLE', '2', 'LAYER', '70', '1',
    '0', 'LAYER', '2', '0', '70', '0', '62', '7',
    '0', 'LAYER', '2', 'WALLS', '70', '0', '62', '1',
    '0', 'ENDTAB', '0', 'ENDSEC',
    '0', 'SECTION', '2', 'ENTITIES',
    '0', 'LINE', '8', 'WALLS', '10', '0', '20', '0', '11', '10', '21', '0',
    '0', 'CIRCLE', '8', 'WALLS', '10', '5', '20', '5', '40', '2',
    '0', 'TEXT', '8', 'NOTES', '10', '1', '20', '1', '40', '2.5', '1', 'Hall',
    '0', 'SPLINE', '8', 'WALLS',
    '0', 'ENDSEC', '0', 'EOF',
].join('\n');

const arcDxf = [
    '0', 'SECTION', '2', 'HEADER', '9', '$INSUNITS', '70', '4', '0', 'ENDSEC',
    '0', 'SECTION', '2', 'TABLES', '0', 'TABLE', '2', 'LAYER', '70', '1',
    '0', 'LAYER', '2', 'WALLS', '70', '0', '62', '1',
    '0', 'ENDTAB', '0', 'ENDSEC',
    '0', 'SECTION', '2', 'ENTITIES',
    '0', 'LINE', '8', 'WALLS', '10', '0', '20', '0', '11', '10', '21', '0',
    '0', 'ARC', '8', 'WALLS', '10', '0', '20', '0', '40', '5', '50', '0', '51', '90',
    '0', 'ENDSEC', '0', 'EOF',
].join('\n');

test('DXF import keeps arcs analytic and reports losses', { skip: withAddon }, async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'open-cad-arc-'));
    const source = path.join(dir, 'arc.dxf');
    fs.writeFileSync(source, arcDxf, 'utf8');
    const imported = service.importDxf({ source, title: 'Arc site' });
    try {
        assert.ok(imported.drawing.entities > 0);
        assert.equal(imported.imported.layers >= 1, true);
        assert.ok(
            !imported.losses.some((loss) => loss.code === 'dxf/flattened-arc'),
            `an analytic arc is not reported as lost, got ${JSON.stringify(imported.losses)}`,
        );
        assert.ok(imported.losses.every((loss) => loss.severity === 'notice' || loss.severity === 'warning'));
    } finally {
        service.close(imported.handle);
        fs.rmSync(dir, { recursive: true, force: true });
    }
});

test('undo, redo, layers, entity edits and DXF import', { skip: withAddon }, async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'open-cad-edits-'));
    const cad = new OpenCAD();
    try {
        const document = await cad.create({ title: 'Edits' });
        assert.equal(document.canUndo, false);
        const line = document.addLine({ layer: '0', from: [0, 0], to: [1, 1] });
        assert.equal(document.canUndo, true);

        const undone = document.undo();
        assert.equal(undone.drawing.entities, 0);
        assert.equal(document.canUndo, false);
        assert.equal(document.canRedo, true);
        assert.throws(
            () => document.undo(),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/nothing-to-undo',
        );
        const redone = document.redo();
        assert.equal(redone.drawing.entities, 1);
        assert.throws(
            () => document.redo(),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/nothing-to-redo',
        );

        const layer = document.addLayer({ name: 'Walls', color: '#a4262c' });
        assert.equal(layer.name, 'Walls');
        assert.throws(
            () => document.addLayer({ name: 'walls' }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/layer-exists',
        );

        const moved = document.moveEntity({ entityId: line.id, dx: 10, dy: 5 });
        if (moved.kind.type === 'line') {
            assert.deepEqual(moved.kind.from, [10, 5]);
            assert.deepEqual(moved.kind.to, [11, 6]);
        }

        const circle = document.addCircle({ layer: '0', center: [0, 0], radius: 3 });
        const updatedCircle = document.updateEntity({ entityId: circle.id, radius: 7 });
        if (updatedCircle.kind.type === 'circle') {
            assert.equal(updatedCircle.kind.radius, 7);
        }
        const text = document.addText({ layer: '0', at: [0, 0], height: 2.5, content: 'Old' });
        const updatedText = document.updateEntity({ entityId: text.id, content: 'Hall', height: 4 });
        if (updatedText.kind.type === 'text') {
            assert.equal(updatedText.kind.content, 'Hall');
            assert.equal(updatedText.kind.height, 4);
        }

        const renamed = document.updateLayer({ layerId: 'Walls', name: 'Structure' });
        assert.equal(renamed.id, 'Structure');
        document.updateLayer({ layerId: 'Structure', locked: true, visible: false });
        assert.throws(
            () => document.addLine({ layer: 'Structure', from: [0, 0], to: [1, 1] }),
            (error: unknown) => error instanceof OpenCADError && error.code === 'drawing/layer-locked',
        );

        const source = path.join(dir, 'site.dxf');
        fs.writeFileSync(source, sampleDxf, 'utf8');
        const imported = await cad.importDxf({ source, title: 'Imported site' });
        assert.equal(imported.entities, 3);
        assert.equal(imported.canUndo, false);
        assert.ok(imported.layers.some((entry) => entry.id === 'WALLS'));
        assert.ok(imported.layers.some((entry) => entry.id === 'NOTES'));

        const pdf = path.join(dir, 'imported.pdf');
        imported.export({ format: 'pdf', target: pdf });
        assert.match(fs.readFileSync(pdf, 'utf8'), /^%PDF-1\.[4-7]/);
        imported.save({ target: path.join(dir, 'imported.ocad'), overwrite: false });
        imported.close();
    } finally {
        await cad.dispose();
        fs.rmSync(dir, { recursive: true, force: true });
    }
});
