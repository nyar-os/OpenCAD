---
name: open-cad
description: Create, revise, inspect and deliver technical drawings with OpenCAD. Use when a task involves drawings, layers, dimensions or CAD exchange formats.
---

# open-cad

## Status

Working document core. The installed CLI and SDK can create and open `.ocad`
drawings, add lines, circles, arcs, ellipses, polylines, text, linear, radial or
diameter and ordinate dimensions,
transform (translate/rotate/scale/mirror), copy, array (rectangular or polar),
explode a polyline and measure, move, update and delete entities, undo and redo,
add, rename, merge, hide and lock layers, save atomically, inspect units, layers,
geometry, polylines, dimensions, duplicate entities, zero-length geometry and
open contours, import the declared ASCII DXF subset (including SPLINE,
LEADER and 3D POLYLINE projection), and export SVG, ASCII DXF, PDF, STL, OBJ or
STEP.
The interactive viewport, the constraint solver, DWG/STEP import, a full DXF
reader and STL/OBJ import are not implemented. Those routes report
`capability/not-implemented`; do not present them as executable and do not
substitute local geometry, parser or drawing code.

## Commands

Run these exact commands. Entity, layer and history commands open the file,
mutate it and save it back to the same path.

```bash
open-cad capabilities

open-cad create --out plan.ocad --title "Site plan" --units mm
open-cad open plan.ocad

open-cad entity add-line plan.ocad --layer 0 --from 0,0 --to 100,40
open-cad entity add-circle plan.ocad --layer 0 --center 50,50 --radius 20
open-cad arc add plan.ocad --layer 0 --center 50,50 --radius 20 --start 0 --end 90
open-cad ellipse add plan.ocad --layer 0 --center 80,20 --rx 30 --ry 10 --rotation 15
open-cad entity add-polyline plan.ocad --layer 0 --points "0,0 100,0 100,60" --closed
open-cad entity add-text plan.ocad --layer 0 --at 5,5 --height 2.5 --text "Site plan"
open-cad entity add-dimension plan.ocad --layer dimensions --from 0,0 --to 100,0 --offset 10
open-cad entity add-radial-dimension plan.ocad --layer dimensions --center 50,50 --radius 20 --angle 45 --offset 3
open-cad entity add-radial-dimension plan.ocad --layer dimensions --center 50,50 --radius 20 --angle 0 --diameter
open-cad entity add-ordinate-dimension plan.ocad --layer dimensions --origin 0,0 --feature 50,20 --axis x --offset 10
open-cad entity move plan.ocad --id entity-1 --dx 10 --dy 5
open-cad entity update plan.ocad --id entity-3 --text "Hall" --height 3
open-cad entity update plan.ocad --id entity-2 --radius 25
open-cad entity delete plan.ocad --id entity-1
open-cad entity list plan.ocad

open-cad transform plan.ocad --ids entity-1,entity-2 --rotate 45 --center 0,0
open-cad transform plan.ocad --layer 0 --translate 10,5
open-cad transform plan.ocad --all --scale 2 --center 0,0
open-cad transform plan.ocad --all --mirror x --mirror-at 0
open-cad copy plan.ocad --ids entity-1 --dx 10 --dy 0 --count 3
open-cad array plan.ocad --ids entity-1 --rows 2 --columns 3 --row-spacing 10 --column-spacing 10
open-cad array plan.ocad --ids entity-1 --center 0,0 --count 6 --angle 360
open-cad explode plan.ocad --id entity-2
open-cad measure plan.ocad

open-cad undo plan.ocad
open-cad redo plan.ocad

open-cad layer add plan.ocad --name Walls --color "#a4262c"
open-cad layer rename plan.ocad --name Walls --to Structure
open-cad layer merge plan.ocad --from Walls --to Structure
open-cad layer visible plan.ocad --name Structure --visible off
open-cad layer lock plan.ocad --name Structure --locked on

open-cad units convert plan.ocad --to inch

open-cad inspect plan.ocad --checks units,layers,geometry,polylines,dimensions,duplicate-entity,zero-length,open-contour
open-cad export plan.ocad --format svg --out plan.svg
open-cad export plan.ocad --format dxf --out plan.dxf
open-cad export plan.ocad --format pdf --out plan.pdf
open-cad export plan.ocad --format stl --out plan.stl
open-cad export plan.ocad --format obj --out plan.obj
open-cad export plan.ocad --format step --out plan.step
open-cad import site.dxf --out site.ocad
```

`--out` on `create` and `import` refuses to replace an existing file unless
`--overwrite` is given. `--format` accepts the declared targets; `svg`, `dxf`,
`pdf`, `stl`, `obj` and `step` are implemented. STL, OBJ and STEP are
export-only: they extrude closed polylines along +Z and do not read STL/OBJ/STEP,
model surfaces or run boolean operations. OBJ writes the same extrusion as
`v`/`f` mesh records. STEP writes ISO-10303-21 AP203-style text as a faceted
BREP (`MANIFOLD_SOLID_BREP`); open profiles are skipped with
`step/open-profile`.

`entity move` takes either `--dx/--dy` or `--to x,y`, never both. A locked layer
refuses add, move, update, transform, copy, array, explode and delete with
`drawing/layer-locked`.

`transform` needs exactly one selection (`--ids a,b`, `--layer id` or `--all`)
and exactly one operation (`--translate dx,dy`, `--rotate degrees --center x,y`,
`--scale factor --center x,y`, or `--mirror x|y --mirror-at <number>`). Mirror
`x` reflects across `y = at` and mirror `y` across `x = at`. `copy` takes
`--ids`, `--dx`, `--dy` and `--count` (1-10000) and returns the fresh ids.
`array` takes `--ids` and exactly one placement: either `--rows/--columns` with
optional `--row-spacing/--column-spacing`, or
`--center x,y --count --angle degrees`. The source stays at its own position, so
a rectangular grid creates `rows * columns - 1` copies and a polar array
`count - 1` copies; both return the fresh ids. `explode` takes `--id` of one
polyline and replaces it, at the same list position, with one line per edge
(closed polylines gain the closing edge). `measure` reports bounds, width,
height, entity count, path length and enclosed area: ellipse length is sampled at
five-degree steps and area only covers closed polylines, full circles and full
ellipses.

`undo` and `redo` use the `<file>.history.json` sidecar that every mutating
command maintains (up to 32 prior documents, atomic writes), so they work across
separate CLI processes. Restoring a snapshot bumps the revision relative to the
current file. An empty sidecar reports `drawing/nothing-to-undo` or
`drawing/nothing-to-redo`. `create` and `import` clear stale history because they
start a new document.

## Drawing model

- A `.ocad` file is UTF-8 JSON tagged `open-cad-drawing`, version `1`.
- Units are `mm` or `inch`. `units convert --to mm|inch` scales every length by
  25.4 or 1 / 25.4 and changes the declared unit. Explicit dimension labels are
  kept as text, so a label that conflicts with the new unit is still reported by
  the `units` check.
- Every drawing starts with layer `0` (colour `#d0d7de`) and layer `dimensions`
  (colour `#0b7285`).
- Entity ids are deterministic: `entity-1`, `entity-2`, ... and are never
  reused after a delete.
- Entities are `line`, `circle`, `arc`, `ellipse`, `point`, `polyline`, `text`,
  `dimension` (linear), `radial_dimension`, `ordinate_dimension`. A radial
  dimension stores a centre, radius, an angle locating the measured point and a
  leader offset; `diameter` switches the measured value from radius to diameter.
  An ordinate dimension stores a datum origin, a feature point, an `x` or `y`
  axis and a perpendicular offset. Arcs store centre, radius and
  counter-clockwise start/end degrees. Ellipses store centre, `rx`/`ry`, a
  major-axis rotation and start/end parameters measured from that axis.
- The revision increments on every mutation. Save does not change it, so a
  reopened file keeps the revision it was written with.
- Each handle keeps up to 32 snapshots. Every mutation adds one step; open and
  create start empty; save does not add a step. Undo and redo restore a snapshot
  and bump the revision.
- Layers are unique by name. Renaming a layer also rewrites every entity
  reference in the same mutation. `layer merge --from A --to B` moves every
  entity of `A` onto `B`, removes the empty `A` and is one history step. Locked
  layers refuse writes; hidden layers are skipped by the exporters.
- `open-cad import` builds a new drawing from the declared ASCII DXF subset:
  `$INSUNITS`, the LAYER table, BLOCKS/INSERT expansion, and LINE, CIRCLE, ARC,
  ELLIPSE, POINT, TEXT, LWPOLYLINE (bulge sampled as arcs),
  POLYLINE/VERTEX/SEQEND (3D polylines are projected onto the XY plane), SPLINE
  (a polyline through the fit points, or the control polygon when there are no
  fit points) and LEADER (an open polyline). Rotated or aligned linear, radial,
  diameter and ordinate DIMENSION references are read. ARC and ELLIPSE stay
  analytic; only polyline bulge arcs are flattened. HATCH, MLEADER and angular
  dimensions are reported as `dxf/unsupported-entity`, and every approximation
  is reported with its own notice code. The import result carries `losses` as
  `{ code, severity, detail }`.

## Inspection

The implemented checks are `units`, `layers`, `geometry`, `polylines`,
`dimensions`, `duplicate-entity`, `zero-length` and `open-contour`. `units` only
warns when a dimension label carries an explicit unit word or inch symbol that
conflicts with the document unit. `duplicate-entity` reports entities whose
layer and geometry are identical; `zero-length` reports lines whose ends
coincide and polylines whose whole length collapses to one point;
`open-contour` reports chains of two or more lines or open polyline segments
that do not close back on themselves (vertices within one millionth of a
drawing unit count as joined).
Findings carry a code, severity, revision, layer id, object id, recoverability
and message. A drawing without error-level findings passes; a locked-layer
finding is a warning.

Reported findings include duplicate entity ids, entities on a missing layer,
zero-length lines, non-positive circle/arc radii and ellipse axes, duplicated
identical entities, polylines with fewer than two points or that are open,
repeated polyline vertices (zero-length segment), non-finite coordinates,
dimensions with identical endpoints or an empty label, radial dimensions with a
non-positive radius, ordinate dimensions measuring zero distance, empty text,
duplicate layer names, and geometry on a hidden or locked layer. `passed` is false only
when an error-level finding exists.

`constraints` and `references` are not implemented. Requesting them fails with
`capability/not-implemented` instead of a false pass.

## Preflight

1. Run `open-cad capabilities` and confirm the export target and checks the task
   needs. A target listed as `unsupported` must not be promised. `dxf` is
   `partial` because only a declared subset imports.
2. Confirm the unit convention, the layer needed and the source file before
   editing. Create the layer with `layer add` or adding an entity to an unknown
   layer fails with `object/not-found`. A locked layer fails writes with
   `drawing/layer-locked`.
3. If a required route is unsupported, stop and report it. Never degrade
   silently or swap in an unrelated tool.

## Routes

| Route | Intent | Evidence |
|-------|--------|----------|
| create | Build an empty drawing, then add measured entities | reopened file, revision, inspection report |
| edit | Change an existing drawing without losing layer or revision identity | moved/updated entity, undo/redo report, saved file, inspection report |
| review | Audit units, layers, geometry and dimensions | per-check findings with layer/object locators |
| deliver | Export SVG, DXF or PDF | written file, loss report, remaining limitations |
| import | Read a declared ASCII DXF subset into a new drawing | imported entity/layer counts, warnings, reopened file |

## Hard rules

- Never export a drawing whose layers are flattened or whose data was altered to
  dodge a fidelity problem.
- Never claim a check ran when it did not, and never fabricate a rendered view.
  The studio viewport is not implemented; SVG, DXF and PDF output is the visual
  deliverable today.
- Describe DXF import as the declared subset only. Do not claim a full DXF
  reader, and do not describe STEP, STL, OBJ, PNG or the constraint solver as
  working.
- Another CAD application may serve only as an optional independent verification
  source. The workflow must not require one.

## Safety

- Confirm with the user before any command that writes or overwrites a drawing
  file, and state the target path first.
- Entity commands save in place. Copy the source file before a destructive edit
  unless the user asked for in-place changes.
- Never delete or re-encode source drawings without explicit confirmation.

## References

- `../../../../crates/cad-model` — document store, persistence, history,
  inspection, layer/entity editing and import entry points.
- `../../../../crates/cad-model` — ASCII DXF subset reader.
- `../../../../crates/cad-model` — SVG, DXF and PDF writers.
- `../../../../crates/cad-model` — drawing, layer and entity schema.
- `projects/packages/open-cad/src/document.ts` — SDK document wrapper.
