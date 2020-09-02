# @wae/open-cad

TypeScript surface for OpenCAD: one drawing model behind inspection, exchange
and agent authoring. The session entry point is `new OpenCAD(options?)`.

## Status

The document core works. `create()`, `open()` and `importDxf()` return a
`DrawingDocument` backed by a native handle; the document exposes `inspect`,
`save`, `export`, `undo`, `redo`, `addLayer`, `updateLayer`, `addLine`,
`addCircle`, `addPolyline`, `addText`, `addDimension`, `moveEntity`,
`updateEntity`, `deleteEntity`, `listEntities` and `close`. Inspection covers
units, layers, geometry and dimensions. SVG, ASCII DXF, vector PDF, ASCII STL
and faceted STEP export are implemented, and the declared ASCII DXF subset can
be imported.

The interactive viewport, DWG/STEP import, a full DXF reader, the STL reader,
the constraint solver and a modelling kernel are not implemented. `capabilities()`
reports them as unsupported, and those routes reject with
`capability/not-implemented`. STEP output is implemented as a faceted BREP
projection of closed profiles.

| Surface   | Status    | Notes                                                             |
| :-------- | :-------- | :---------------------------------------------------------------- |
| `.ocad`   | partial   | JSON project, bounded undo/redo history                           |
| DXF       | partial   | import subset below; ASCII R12 export                             |
| SVG / PDF | supported | read-only preview and vector page                                 |
| STL       | supported | export only; closed profiles extruded along +Z                    |
| STEP      | partial   | export only; faceted BREP of closed profiles                      |
| DWG       | unsupported | `capability/not-implemented`                                    |

### DXF import subset

`$INSUNITS` (1 inch, 4 mm, 5 cm, 6 m; missing defaults to mm), the LAYER table
(ACI colours approximated as RGB), BLOCKS with `INSERT` (point, scale, rotation,
nested up to 8 levels) and the entities LINE, CIRCLE, ARC, ELLIPSE, POINT, TEXT,
LWPOLYLINE (bulge sampled as arcs) and POLYLINE/VERTEX/SEQEND. ARC and ELLIPSE
become polylines at 5° per segment (at least 8 segments) and are reported with
`dxf/flattened-arc` / `dxf/flattened-ellipse` notices. Each unsupported entity
type is reported once with `dxf/unsupported-entity`, including an example handle
and source line.

### STL export limits

STL export extrudes every **closed** polyline along +Z (default thickness 1.0
in drawing units) and triangulates the caps with ear clipping. It is not a
surface or solid modeller: it does no boolean operations, does not extrude
circles, text or dimensions, skips open profiles with `stl/open-profile`, and
skips degenerate triangles with `stl/degenerate-triangle`.

### STEP export limits

`export('step', path)` writes ISO-10303-21 AP203-style text and reuses the same
closed-profile extrusion and ear-clipping triangulation as STL. Each triangle is
one planar `ADVANCED_FACE` over a `POLY_LOOP`, and each closed profile becomes
its own `CLOSED_SHELL` inside a `MANIFOLD_SOLID_BREP`, so disjoint profiles stay
separate solids. It is a faceted BREP projection, not a solid modeller: no
booleans, analytic surfaces or features. Circles, text and dimensions are not
extruded, open profiles are skipped with `step/open-profile`, and the report
carries the `faces` count. STEP import is not implemented.

A `history: 'sidecar'` session also records each replaced document in
`<file>.history.json` (up to 32 steps, atomic writes), so `undo()` and `redo()`
work across separate processes; the CLI uses this mode. A restore bumps the
revision relative to the current file, and `save({ target })` to a new target
adopts it as the source path and clears the target's stale history. The default
`history: 'memory'` keeps snapshots inside the native handle only.

```ts
const cad = new OpenCAD();
const drawing = await cad.create({ title: 'Site plan', units: 'mm' });
drawing.addLine({ layer: '0', from: [0, 0], to: [100, 40] });
drawing.addCircle({ layer: '0', center: [50, 50], radius: 20 });
drawing.addLayer({ name: 'Walls', color: '#a4262c' });
drawing.moveEntity({ entityId: 'entity-1', dx: 10, dy: 5 });
drawing.updateEntity({ entityId: 'entity-2', radius: 25 });
drawing.undo();
drawing.redo();
const report = drawing.inspect();
drawing.save({ target: 'plan.ocad' });
drawing.export({ format: 'pdf', target: 'plan.pdf' });
const imported = await cad.importDxf({ source: 'site.dxf' });
await cad.dispose();
```

`new OpenCAD({ history: 'sidecar' })` opts into the file-backed history. The
`history` option is read once by the session; every document it opens or creates
inherits it.

## Boundaries

- No drawing engine in this package: the native addon calls the shared Rust
  model crate in `../../crates/cad-model`.
- No DWG or STEP importer, and no full DXF reader. `importDxf()` reads the
  subset listed above; every approximation and skip is returned in `losses` with
  `{ code, severity, detail }`.
- No STEP, PNG or STL reader. PDF, SVG, DXF, ASCII STL and STEP writers exist.
- No viewport renderer, constraint solver or layer deletion.
- Native access goes through the Node-API binding in `../../crates/cad-napi`.

## Scripts

```bash
pnpm --filter @wae/open-cad run typecheck
pnpm --filter @wae/open-cad run build
pnpm --filter @wae/open-cad run test
```
