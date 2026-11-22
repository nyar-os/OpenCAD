# OpenCAD

2D technical drawings that live in plain text and reproduce from the command line.

## 💡 What is OpenCAD?

OpenCAD is a 2D technical drawing tool. You describe a drawing as a `.ocad` JSON
document — layers with colours, line types and lineweights, geometry, text,
hatches, dimensions, groups and paper-space layouts — then edit, audit and
export it from the `open-cad` command line or from a TypeScript session. The
same document is what the command line edits, what the SDK reads and what the
exporter projects, so a script and a person always see the same state.

Typical uses:

- review drawings like code — the document is text, so a diff shows exactly which
  layer, entity or style changed;
- automate repetitive edits from a script or an agent, with stable entity ids and
  stable error codes instead of screenshot-and-click instructions;
- produce an SVG or PDF preview or plot without a desktop CAD application;
- bring geometry, layers and basic dimensions across from an existing DXF file.

**OpenCAD is not:**

- a DWG editor or a general CAD kernel;
- a constraint solver, parametric sketcher or 3D solid modeller;
- an interactive viewport — SVG output is the preview;
- a full DXF reader or writer — import covers a declared ASCII subset, and
  STEP/STL/OBJ export are faceted projections rather than analytic solids.

A drawing is one document, not a pile of files: layers, styles, layouts, groups
and draw order travel together, and every edit produces a new revision.

## 🚀 Getting started

### 1. Point your agent at the skill

```bash
pnpm add @wae/open-cad-skills
```

Point your agent at
`node_modules/@wae/open-cad-skills/skills/open-cad/SKILL.md`, then give it a
concrete task:

> Use OpenCAD (`open-cad`) to create `studio.ocad` in millimetres with a
> 12,000 × 8,000 room on a `Walls` layer, put the title "Studio" inside it, add
> an overall dimension on the `dimensions` layer, inspect the drawing and export
> `studio.svg`. Read `documentation/capabilities.md` before claiming a result.

The skill lists the verified commands, their order and the checks to run before
export, so the agent does not have to guess a workflow.

### 2. Install and run

```bash
pnpm add @wae/open-cad
```

```bash
open-cad create --out plan.ocad --title "Studio plan" --units mm
open-cad layer add plan.ocad --name Structure --color "#a4262c"
open-cad entity add-rectangle plan.ocad --layer Structure --from 0,0 --to 12000,8000
open-cad entity add-text plan.ocad --at 600,600 --height 300 --text "Studio"
open-cad entity add-dimension plan.ocad --from 0,0 --to 12000,0 --offset 600
open-cad inspect plan.ocad
open-cad export plan.ocad --format svg --out plan.svg
```

`inspect` exits non-zero when a check fails. `export` writes SVG at the drawing's
extents; use `--format pdf`, `dxf`, `dxf-r12`, `stl`, `obj` or `step` for another
target. `open-cad capabilities` lists what the installed build supports, and
`open-cad --help` lists every command.

The SDK is the same engine with a typed API:

```ts
import { OpenCAD } from '@wae/open-cad';

const cad = new OpenCAD();
const drawing = await cad.create({ title: 'Studio plan', units: 'mm' });
drawing.addLayer({ name: 'Walls', color: '#a4262c' });
drawing.addRectangle({ layer: 'Walls', from: [0, 0], to: [12000, 8000] });
drawing.export({ format: 'svg', target: 'plan.svg' });
drawing.save({ target: 'plan.ocad' });
await cad.dispose();
```

## 🧭 When OpenCAD fits

- You want drawings reviewed like code, with a readable diff for every change.
- You want an agent or a script to make the same edit repeatably.
- You need a preview or a plot without opening a desktop CAD application.
- You already have DXF drawings and need only their geometry, layers and basic
  dimensions.
- You need full DWG fidelity, an interactive canvas or a constraint solver —
  those are deliberately outside this tool.

## 🗂️ Working with existing drawings

`import` reads a declared ASCII DXF subset and returns every approximation it
made. Notices mean geometry was kept with a documented change; warnings mean
something was skipped. Nothing is dropped silently.

```bash
open-cad import survey.dxf --out survey.ocad
open-cad inspect survey.ocad
open-cad export survey.ocad --format pdf --out survey.pdf
```

The import report names each loss by code and detail, so you can decide whether
the subset is enough before you build on the result. The exact subset and loss
codes are listed in [Capabilities](documentation/capabilities.md).

## ✨ Highlights

- **Layers that carry drawing intent.** Colour, visibility, lock, freeze, plot,
  line type and lineweight, with lineweights and dash patterns rendered in SVG
  and PDF.
- **Geometry and dimensions.** Lines, points, circles, arcs, ellipses, polylines,
  splines, rectangles, polygons, text and hatches, plus linear, radial, ordinate
  and angular dimensions with named dimension styles.
- **Edit and transform.** Move, copy, rectangular and polar arrays, rotate,
  scale, mirror, offset, fillet, chamfer, explode, and per-vertex polyline
  editing.
- **Undo and redo.** A 32-step history on every document; the command line also
  keeps it beside the drawing, so undo works across separate calls.
- **Audit before you ship.** Inspection checks units, layers, geometry,
  polylines, dimensions, duplicates, zero-length entities and open contours, plus
  per-entity and drawing-wide measurements.
- **Object snap.** A probe point resolves to `endpoint`, `midpoint`, `center`,
  `quadrant`, `nearest`, `intersection`, `perpendicular` and `tangent`
  candidates, sorted by distance with an optional tolerance.
- **Exchange.** Declared ASCII DXF import with explicit loss notices, and DXF,
  SVG, PDF, STL, OBJ and STEP export.
- **Agent authoring.** A skills package with a verified command list, so an agent
  can build, audit and export a drawing without trial and error.

## 🧪 Examples

- [Package README](projects/packages/open-cad/README.md) — install, quick start,
  the API surface by task and the constraints.
- [Smoke test](projects/packages/open-cad/tests/smoke.test.ts) — runnable
  evidence for the SDK surface, undo, import and export.
- [CLI reference](documentation/cli.md) — every command with flags and examples.
- [Capabilities](documentation/capabilities.md) — the support matrix, the DXF
  subset and every unsupported boundary.

## 📚 Documentation

| Document | Contents |
|----------|----------|
| [documentation/architecture.md](documentation/architecture.md) | Drawing model, module boundaries and the native call path |
| [documentation/capabilities.md](documentation/capabilities.md) | Support matrix, DXF subset, loss codes and boundaries |
| [documentation/cli.md](documentation/cli.md) | Every `open-cad` command and flag |
| [documentation/development.md](documentation/development.md) | Build, test, native staging and release |
| [projects/packages/open-cad/README.md](projects/packages/open-cad/README.md) | The npm package page |

## 📜 License

[Apache-2.0](License.md)
