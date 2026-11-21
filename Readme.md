OpenCAD workspace
=================

OpenCAD is a drawing workspace built around one model of a drawing: layers,
geometry, dimensions and references. This repository holds the product surface
around that model — the command-line tool and SDK, the studio window, the agent
skills package and the product website.

Status
------

Working document core. A `.ocad` project can be created, opened, edited, saved
and reopened; lines, circles, arcs, ellipses, polylines, text, linear
dimensions, radial/diameter dimensions and ordinate dimensions are real
entities; transform (translate/rotate/scale/mirror), copy,
rectangular/polar array, polyline explode and measure are supported; each handle
keeps a 32-snapshot undo/redo history;
layers can be added, renamed, merged, hidden and locked; a drawing converts
between millimetres and inches by scaling every length; inspection reports
per-object findings;
SVG, ASCII DXF, vector PDF, ASCII STL, Wavefront OBJ and STEP export work, and
a declared ASCII DXF subset can be imported. The interactive viewport, DWG/STEP
import, a full DXF reader, the STL and OBJ readers, the constraint solver and a
modelling kernel are not implemented and return the stable
`capability/not-implemented` code. STEP output is a faceted BREP projection of
closed profiles, not a solid model; OBJ output is the same extrusion as a mesh.

What is here
------------

| Surface        | Package                        | Purpose                                                              |
| :------------- | :----------------------------- | :------------------------------------------------------------------- |
| SDK and CLI    | `@wae/open-cad`                | `new OpenCAD()` session, handle-backed document and `open-cad` command |
| Studio         | `@wae/open-cad-ui`             | Desktop window with undo/redo, layer controls, import and inspector  |
| Skills         | `@wae/open-cad-skills`         | Agent routing and executable drawing commands                         |
| Website        | `@wae/open-cad-homepage`       | Product pages: overview, features, roadmap                            |
| Platform addon | `@wae/open-cad-<platform>-<arch>` | Prebuilt binaries for Windows, macOS and Linux                   |

Try the command line
--------------------

```bash
open-cad capabilities
open-cad create --out plan.ocad --title "Site plan" --units mm
open-cad entity add-line plan.ocad --layer 0 --from 0,0 --to 100,40
open-cad entity add-circle plan.ocad --layer 0 --center 50,50 --radius 20
open-cad entity add-radial-dimension plan.ocad --layer dimensions --center 50,50 --radius 20 --angle 45 --offset 3
open-cad entity add-radial-dimension plan.ocad --layer dimensions --center 50,50 --radius 20 --angle 0 --diameter
open-cad entity add-ordinate-dimension plan.ocad --layer dimensions --origin 0,0 --feature 50,20 --axis x --offset 10
open-cad arc add plan.ocad --layer 0 --center 50,50 --radius 20 --start 0 --end 90
open-cad ellipse add plan.ocad --layer 0 --center 80,20 --rx 30 --ry 10 --rotation 15
open-cad entity move plan.ocad --id entity-1 --dx 10 --dy 5
open-cad transform plan.ocad --ids entity-1,entity-2 --rotate 45 --center 0,0
open-cad transform plan.ocad --layer 0 --translate 10,5
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
open-cad layer lock plan.ocad --name Structure --locked on
open-cad units convert plan.ocad --to inch
open-cad entity list plan.ocad
open-cad inspect plan.ocad --checks units,layers,geometry,polylines,dimensions,duplicate-entity,zero-length,open-contour
open-cad export plan.ocad --format svg --out plan.svg
open-cad export plan.ocad --format dxf --out plan.dxf
open-cad export plan.ocad --format pdf --out plan.pdf
open-cad export plan.ocad --format stl --out plan.stl
open-cad export plan.ocad --format obj --out plan.obj
open-cad export plan.ocad --format step --out plan.step
open-cad import site.dxf --out site.ocad
```

Entity, layer and history commands open the document, mutate it and let the
SDK write it back. `new OpenCAD({ history: 'sidecar' })` records the previous
document in a `<file>.history.json` sidecar (up to 32 steps, atomic writes), so
`open-cad undo` and `open-cad redo` keep working across separate CLI processes;
a restore bumps the revision relative to the current file. `create` and `import`
save through the SDK, which adopts the new target and clears its stale sidecar.
The default `history: 'memory'` session keeps snapshots inside the native handle
only. The CLI uses only the session and document methods (`capabilities`,
`create`, `open`, `importDxf`, `inspect`, `save`, `export`, `undo`, `redo`,
entity and layer editing, `close`); it has no direct `service` access. The studio
window is opened with `open-cad gui`; it reads documents and capabilities from
the native host, but it does not paint geometry yet.

What this build does NOT do
---------------------------

- No DWG or STEP import, and no full DXF reader. `import` accepts the declared
  ASCII subset only: `$INSUNITS`, the LAYER table, BLOCKS/INSERT expansion, the
  ATTRIB values that follow an INSERT, and LINE, CIRCLE, ARC, ELLIPSE, POINT,
  TEXT, LWPOLYLINE (including bulge), POLYLINE/VERTEX/SEQEND (a 3D polyline is
  projected onto the XY plane), SPLINE (read as a polyline through its fit
  points, or its control polygon when no fit points exist) and LEADER (read as
  an open polyline). Rotated or aligned linear DIMENSION references are read.
  ARC and ELLIPSE stay analytic, and only bulge arcs are flattened; rotated or
  aligned linear, radial, diameter and ordinate dimensions are read, while
  angular dimensions, HATCH and MLEADER are skipped.
  Every approximation and skip is reported as a loss with a stable code, a
  severity (`notice` or `warning`) and a detail.
- No STEP or OBJ reader, and no PNG writer, and no STL reader. STL and OBJ
  export extrude closed profiles along +Z (default thickness 1.0 drawing units);
  arcs and ellipses are sampled at five-degree steps, and a full ellipse is a
  closed profile. OBJ writes the same triangles as `v`/`f` records. Neither is a
  surface or solid modeller and neither performs boolean operations. Open
  profiles are skipped with `stl/open-profile` or `obj/open-profile`; degenerate
  triangles are skipped with `stl/degenerate-triangle` or
  `obj/degenerate-triangle`. PDF export exists as a minimal vector page and
  approximates arcs and ellipses with cubic Bezier segments; SVG and DXF export
  also work.
- STEP export (`export --format step`) writes ISO-10303-21 AP203-style text and
  reuses the same closed-profile extrusion and ear-clipping triangulation as
  STL. Every triangle becomes one planar `ADVANCED_FACE`, and each closed
  profile becomes its own faceted solid (`CLOSED_SHELL` inside a
  `MANIFOLD_SOLID_BREP`), so disjoint profiles stay separate; there are no
  booleans, analytic surfaces or features. Open profiles are skipped with
  `step/open-profile` and never become a fabricated solid. STEP import is not
  implemented.
- `units convert --to mm|inch` scales every coordinate, radius, ellipse axis,
  text height and dimension offset by 25.4 or 1 / 25.4. Explicit dimension label
  text is kept verbatim, so the `units` check still reports a label that
  conflicts with the new unit.
- No interactive viewport, selection or snapping.
- No constraint solver or associative dimensions; `constraints` and `references`
  checks are reported as unsupported.
- No bare layer deletion. `layer merge --from A --to B` moves every entity of
  `A` onto `B` and removes the now-empty `A` in one history step. A locked
  source or target layer refuses the merge.
- `capabilities()` reports `project: partial`, SVG/PDF/STEP/STL/OBJ export as
  supported, DXF as partial because of its import subset, STEP and OBJ as
  partial because only their export direction is implemented, STL import and DWG
  as unsupported, and the unimplemented routes as `unsupported`. Its `entities`
  matrix lists line, circle, arc, ellipse, point, polyline, text, dimension,
  radial dimension and ordinate dimension as supported; its
  `operations` matrix lists transform, copy, array, explode and measure as
  supported while constraint solving and a 3D kernel stay unsupported. A
  rectangular `array` of `rows * columns` cells creates that many minus one
  copies (the source stays in place); a polar array of `count` items does the
  same. `explode` replaces one polyline with independent line entities. Document
  checks: `units`, `layers`, `geometry`, `polylines`, `dimensions`,
  `duplicate-entity`, `zero-length` and `open-contour` run; `constraints` and
  `references` stay unsupported. `units` is conservative: it
  warns only when a dimension's explicit label carries a unit word or inch
  symbol that conflicts with the document unit.
- `measure` sums line segments, polyline edges and analytic arc lengths; ellipse
  length is sampled at five-degree steps, so it is approximate. Area covers
  closed polylines (shoelace), full circles and full ellipses; partial curves
  and self-intersecting polylines are not exact.
