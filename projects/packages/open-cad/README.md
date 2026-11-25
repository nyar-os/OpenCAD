# @wae/open-cad

The OpenCAD TypeScript SDK and the `open-cad` command line for 2D technical
drawings. Both edit the same `.ocad` document.

## Install

```bash
pnpm add @wae/open-cad
```

The native addon is resolved automatically for the host platform. Set
`OPEN_CAD_NATIVE` to the absolute path of a `.node` file to use a local build
instead. A missing addon is not a crash: engine calls report
`capability/not-implemented`.

## Quick start

```ts
import { OpenCAD } from '@wae/open-cad';

const cad = new OpenCAD();
const drawing = await cad.create({ title: 'Site plan', units: 'mm' });
drawing.addLayer({ name: 'Walls', color: '#a4262c' });
drawing.addLine({ layer: 'Walls', from: [0, 0], to: [100, 40] });
drawing.addCircle({ layer: '0', center: [50, 50], radius: 20 });

const report = drawing.inspect();
if (!report.passed) {
    console.log(report.issues);
}

drawing.export({ format: 'svg', target: 'plan.svg' });
drawing.save({ target: 'plan.ocad' });
await cad.dispose();
```

The command line uses the same document:

```bash
open-cad create --out plan.ocad --title "Site plan" --units mm
open-cad entity add-line plan.ocad --layer 0 --from 0,0 --to 100,40
open-cad export plan.ocad --format svg --out plan.svg
```

## API surface by task

The package exports `OpenCAD` (the session), `DrawingDocument` (one open
drawing), `capabilities`, the format registry, error classes and all public
types.

| Task | Methods |
| :--- | :--- |
| Session | `cad.capabilities()`, `cad.create()`, `cad.open()`, `cad.importDxf()`, `cad.importSvg()`, `cad.inspect()`, `cad.dispose()` |
| Draw | `addLine`, `addPoint`, `addCircle`, `addArc`, `addEllipse`, `addPolyline`, `addSpline`, `addRectangle`, `addPolygon`, `addText`, `addHatch`, `addDimension`, `addRadialDimension`, `addOrdinateDimension`, `addAngularDimension` |
| Edit | `moveEntity`, `updateEntity`, `updateArc`, `updateEllipse`, `updateDimension`, `updatePolyline`, `updateSpline`, `assignEntityLayer`, `deleteEntity`, `transform`, `copy`, `array`, `explode`, `offset`, `offsetBySide`, `trim`, `extend`, `breakEntity`, `joinEntities`, `fillet`, `chamfer`, `polylineChamfer`, `reorderEntity` |
| Layers and styles | `addLayer`, `updateLayer`, `mergeLayer`, `deleteLayer`, `addLinetype`, `updateLinetype`, `deleteLinetype`, `addDimStyle`, `updateDimStyle`, `deleteDimStyle`, `addTextStyle`, `updateTextStyle`, `deleteTextStyle` |
| Groups and layouts | `createGroup`, `addGroupMembers`, `removeGroupMembers`, `deleteGroup`, `createLayout`, `updateLayout`, `deleteLayout` |
| Inspect and measure | `inspect`, `stats`, `measure`, `measureEntities`, `measurePoints`, `select`, `snap`, `ucs`, `findText`, `replaceText` |
| History | `undo`, `redo`, `close` |
| Save and export | `save`, `export` |

`export({ format })` accepts `dxf`, `dxf-r12`, `svg`, `pdf`, `stl`, `obj`,
`step` and `iges`. The result reports what the target kept in `losses`, so a
successful export does not imply a lossless file. `convertUnits` switches a drawing between
`mm` and `inch`.

## Constraints

- DXF import covers a declared ASCII subset. Every approximation is returned as a
  loss with a stable code; unsupported entity types are skipped with a warning.
- SVG import covers a declared element subset (`line`, `circle`, `ellipse`,
  `rect`, `polyline`, `polygon`, `path`, `text`); transforms and styling are
  not applied and path curves are sampled.
- No interactive viewport, DWG reader, full DXF reader or constraint solver.
  `capabilities()` reports these as unsupported, and operations that need them
  answer `capability/not-implemented`.
- STEP, STL and OBJ are export-only faceted projections, not analytic solids.
- `history: 'sidecar'` is required for undo across processes; it is what the CLI
  uses. The default keeps history in memory.
- Colours are `#rrggbb`. Layer lineweights are `0`–`10` mm. A line-type pattern
  holds at most 12 finite positive entries.

The full support matrix and every boundary are documented in
[Capabilities](../../../documentation/capabilities.md). For the product overview
start at the [root README](../../../README.md).
