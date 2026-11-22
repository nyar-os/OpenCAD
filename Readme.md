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
dimensions, radial/diameter dimensions, ordinate dimensions, angular dimensions and hatches are real
entities; transform (translate/rotate/scale/mirror), copy,
rectangular/polar array, polyline explode, offset, fillet, chamfer and measure are supported;
a polyline's vertices can be replaced, inserted, removed and moved and its
closed flag toggled in one history step;
a selected entity or set of entities can be measured for its path length and
enclosed area; two points can be measured for distance and three points for the
angle at the middle vertex; each handle
keeps a 32-snapshot undo/redo history;
layers can be added, renamed, merged, hidden, locked, frozen and marked as
non-plottable; a drawing keeps named
dimension styles (scale, arrowhead size, text height, extension length, origin
offset and label precision) that can be listed, added, updated and removed and
round-trip through the DXF `DIMSTYLE` table; named text styles (font, fixed
height, width factor and oblique angle) round-trip through the DXF `STYLE`
table and can be listed, added, updated and removed; named line types (dash
pattern) round-trip through the DXF `LTYPE` table and each layer carries a line
type id and a lineweight; a rectangular window or
crossing query returns the matching entity ids without adding a history step;
text content can be found and replaced in bulk (ASCII case-insensitive by
default) in one history step; entities can be collected into named groups;
drawing title-block properties (title, author, company, project, description and
keywords) can be read and written; endpoint, midpoint, centre and line
intersection snap points can be computed near a probe point;
a drawing converts
between millimetres and inches by scaling every length; points and entities can
be converted between world coordinates and a user coordinate system defined by
an origin and an X-axis angle; inspection reports
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
open-cad entity add-angular-dimension plan.ocad --layer dimensions --center 0,0 --first 10,0 --second 0,10 --offset 5
open-cad entity add-hatch plan.ocad --layer fills --points "0,0 80,0 80,60 0,60" --pattern ANSI31
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
open-cad offset plan.ocad --id entity-2 --distance 2
open-cad fillet plan.ocad --first entity-1 --second entity-2 --radius 5
open-cad chamfer plan.ocad --first entity-1 --second entity-2 --distance-a 3 --distance-b 3
open-cad measure plan.ocad
open-cad measure plan.ocad --ids entity-1,entity-2
open-cad measure-points plan.ocad --points "0,0 3,4"
open-cad measure-points plan.ocad --points "0,10 0,0 5,0"
open-cad stats plan.ocad
open-cad select plan.ocad --box 0,0,100,100
open-cad select plan.ocad --box 0,0,100,100 --crossing --layer Walls
open-cad snap plan.ocad --modes endpoint,midpoint,center,intersection --near 10,0 --tolerance 5
open-cad undo plan.ocad
open-cad redo plan.ocad
open-cad layer add plan.ocad --name Walls --color "#a4262c"
open-cad layer rename plan.ocad --name Walls --to Structure
open-cad layer merge plan.ocad --from Walls --to Structure
open-cad layer lock plan.ocad --name Structure --locked on
open-cad layer freeze plan.ocad --name Structure --frozen on
open-cad layer plot plan.ocad --name Structure --plottable off
open-cad layer set plan.ocad --name Structure --linetype DASHED --lineweight 0.5
open-cad linetype list plan.ocad
open-cad linetype add plan.ocad --name DASHED --description "Dashed" --pattern 5,2
open-cad linetype update plan.ocad --name DASHED --to DASH-1 --pattern 6,2
open-cad linetype delete plan.ocad --name DASH-1
open-cad dimstyle list plan.ocad
open-cad dimstyle add plan.ocad --name ARCH --scale 2.5 --arrow-size 3 --text-height 3.5 --precision 2
open-cad dimstyle update plan.ocad --name ARCH --to ARCH-1 --scale 3
open-cad dimstyle delete plan.ocad --name ARCH-1
open-cad textstyle list plan.ocad
open-cad textstyle add plan.ocad --name NOTES --font romans.shx --width-factor 0.8 --oblique 10
open-cad textstyle update plan.ocad --name NOTES --to NOTES-1 --height 3
open-cad textstyle delete plan.ocad --name NOTES-1
open-cad text find plan.ocad --pattern hall
open-cad text replace plan.ocad --pattern hall --to "main hall"
open-cad group list plan.ocad
open-cad group create plan.ocad --name Walls --ids entity-1,entity-2
open-cad group add plan.ocad --name Walls --ids entity-3
open-cad group remove plan.ocad --name Walls --ids entity-3
open-cad group delete plan.ocad --name Walls
open-cad properties show plan.ocad
open-cad properties set plan.ocad --title "Harbour plan" --author "Design team" --company OpenCAD --project Harbour
open-cad units convert plan.ocad --to inch
open-cad ucs plan.ocad --origin 10,0 --rotation 90 --direction to-world --points "0,0 10,0"
open-cad ucs plan.ocad --origin 0,0 --rotation 30 --direction to-ucs --ids entity-1,entity-2
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
  ASCII subset only: `$INSUNITS`, the LAYER, LTYPE, STYLE and DIMSTYLE tables, BLOCKS/INSERT expansion, the
  ATTRIB values that follow an INSERT, and LINE, CIRCLE, ARC, ELLIPSE, POINT,
  TEXT, LWPOLYLINE (including bulge), POLYLINE/VERTEX/SEQEND (a 3D polyline is
  projected onto the XY plane), SPLINE (read as a polyline through its fit
  points, or its control polygon when no fit points exist), LEADER (read as
  an open polyline), MLEADER (leader vertices as an open polyline plus one
  TEXT entity per inline text line) and HATCH (one closed boundary path).
  Rotated or aligned linear DIMENSION references are read.
  ARC and ELLIPSE stay analytic, and only bulge arcs are flattened; rotated or
  aligned linear, radial, diameter, ordinate and three-point angular dimensions
  are read; two-line angular dimensions (DXF type 2) are skipped. A HATCH boundary made of line,
  arc or polyline edges becomes a closed polygon; elliptical and spline edges
  skip the entity, and the pattern name is kept as a description without a
  pattern definition. MLEADER inline formatting is removed, its lines are
  stacked as separate `TEXT` entities and leader style, landing, arrow and
  background are not modelled.
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
  also work. DXF export writes one HATCH record per hatch with a closed polyline
  boundary path and, when the drawing holds a hatch, declares `$ACADVER`
  `AC1015` instead of `AC1009`. A pattern name is stored on the record but no
  pattern definition is emitted, so non-solid patterns are also reported as
  `dxf/hatch-pattern`. SVG and PDF draw the hatch boundary polygon.
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
- `fillet` and `chamfer` accept only two straight lines. Arc/line or polyline
  corners, variable radius, extending a segment to reach the trim point and 3D
  edges are not implemented.
- MLEADER text is read only when it is inline context data. Text stored in an
  external MTEXT object is not resolved, and leader style, landing, arrow,
  background and column layout are not modelled.
- No interactive viewport. Snapping is a read-only query over endpoints,
  midpoints, centres and straight-line intersections; arcs and circles do not
  contribute curve-curve intersections, and no snapping feedback is painted.
- No constraint solver or associative dimensions; `constraints` and `references`
  checks are reported as unsupported.
- Dimension styles keep only the modelled `DIMSTYLE` variables: overall scale,
  arrowhead size, text height, extension line length, origin offset and decimal
  places. Other variables are ignored on import and are not written, and
  dimension entities do not bind a named style yet.
- Text styles keep only the primary font, fixed height, width factor and
  oblique angle. Big-font, generation-flag and per-text style references are
  not modelled.
- `text find` is a read-only query and `text replace` is one history step. Both
  scan `TEXT` entities only, not dimension labels or attributes, and matching is
  ASCII case-insensitive unless `--match-case` is passed. A replacement on a
  locked layer refuses the whole edit.
- Selection is a rectangular query. There is no interactive viewport, fence or
  lasso selection, and curves are tested against their sampled outlines.
  Locked and hidden layers stay selectable unless `--layer` narrows the query.
- No bare layer deletion. `layer merge --from A --to B` moves every entity of
  `A` onto `B` and removes the now-empty `A` in one history step. A locked or
  frozen source or target layer refuses the merge.
- `capabilities()` reports `project: partial`, SVG/PDF/STEP/STL/OBJ export as
  supported, DXF as partial because of its import subset, STEP and OBJ as
  partial because only their export direction is implemented, STL import and DWG
  as unsupported, and the unimplemented routes as `unsupported`. Its `entities`
  matrix lists line, circle, arc, ellipse, point, polyline, text, dimension,
  radial dimension, ordinate dimension, angular dimension and hatch as supported; its
  `operations` matrix lists transform, copy, array, explode, offset, fillet,
  chamfer, measure, measure-entity, measure-points, stats, dimension-style,
  text-style, text-search, group, select, properties, snap, linetype and ucs as
  supported while constraint solving and a 3D kernel stay unsupported. A
  rectangular `array` of `rows * columns` cells creates that many minus one
  copies (the source stays in place); a polar array of `count` items does the
  same. `explode` replaces one polyline with independent line entities.
  `offset` appends a new circle, arc or polyline: it grows or shrinks a radius,
  or offsets polyline vertices along mitered left/right normals. `fillet` and
  `chamfer` work on one pair of lines: the closest endpoints define the corner,
  both lines are trimmed towards their far endpoints, and a tangent arc or a
  bevel line is appended on the first line's layer. A radius or trim distance
  that would leave either segment, and parallel, collinear, zero-length or
  non-line references, are refused with `session/argument-invalid` or
  `object/unsupported`. Document
  checks: `units`, `layers`, `geometry`, `polylines`, `dimensions`,
  `duplicate-entity`, `zero-length` and `open-contour` run by default;
  `empty-layer` is opt-in and warns about layers that hold no entities.
  `constraints` and `references` stay unsupported. `units` is conservative: it
  warns only when a dimension's explicit label carries a unit word or inch
  symbol that conflicts with the document unit.
- `measure` sums line segments, polyline edges, hatch boundaries and analytic
  arc lengths; ellipse length is sampled at five-degree steps, so it is
  approximate. Area covers closed polylines and hatches (shoelace), full
  circles and full ellipses; partial curves and self-intersecting polylines are
  not exact.
- Line types store a name, description and dash pattern and a drawing always
  keeps at least one. Layers carry a line type id and a lineweight in
  millimetres; both round-trip through the DXF `LTYPE` and LAYER tables
  (lineweight as code 370). SVG strokes every layer at its lineweight and
  reduces the line type's dash pattern to one dash/gap pair
  (`stroke-dasharray`); a longer pattern averages its dash entries and its gap
  entries, and a zero-length dash or gap keeps the stroke solid. PDF strokes at
  the same lineweight but keeps lines solid because its scene backend has no
  dash surface. A zero lineweight keeps the exporter default. Per-entity
  overrides are not modelled.
- Drawing properties are stored in the `.ocad` document only; the DXF header,
  title-block geometry and sheet attributes are not written or read. Values are
  trimmed, empty values clear a field, and each value is limited to 512
  characters.
- Entity groups are named membership lists stored in the `.ocad` document only;
  the DXF `GROUP` table is not read or written. Grouping never changes geometry,
  and deleting or exploding a member removes its id from every group.
- `measure --ids entity-1,entity-2` reports `length`, `area` and `closed` per
  entity plus totals, using the same rules as the drawing-wide `measure`. A
  closed polyline reports its perimeter as `length`; circle circumference is not
  included, matching `measure`.
- `measure-points` is a read-only query. Two points report the distance, the
  signed `dx`/`dy` offsets and a bearing in degrees; three points treat the
  middle point as the vertex and report the angle between the legs in degrees,
  both leg lengths and the span between the first and third points. A repeated
  vertex is refused with `session/argument-invalid`.
- A layer can be frozen: frozen layers are skipped by the SVG and PDF output and
  refuse every write with `drawing/layer-frozen`. A layer can also be marked
  non-plottable, which omits it from plots while the drawing keeps its geometry.
  Both flags round-trip through the DXF LAYER table as code 70 flag 1 and code
  290, and a drawing with a non-plottable layer is written as `AC1015`.
- `ucs` converts between world coordinates and a user coordinate system defined
  by an origin and an X-axis angle. `--points` reports each conversion without a
  history step. `--ids` applies the same rotation and translation to entities in
  one history step, respecting locked and frozen layers. Named UCS entries, a
  rotating view and a per-viewport UCS are not modelled.
- `stats` groups the same length and area by layer, adds per-layer entity counts
  and drawing-wide totals, so an audit can tell which layer owns the geometry.
