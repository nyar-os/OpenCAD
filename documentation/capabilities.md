# Capabilities

The capability matrix is the only support claim in this repository. Text in a
README, a skill or a prompt is not evidence of support; `capabilities()` is.

`capabilities()` (CLI: `open-cad capabilities`, SDK: `cad.capabilities()`)
reports five maps:

- `project` — the native `.ocad` project: `partial`;
- `formats` — `supported`, `partial` or `unsupported` for each exchange format;
- `checks` — inspector checks;
- `renderers` — preview renderers (only `svg`);
- `exportTargets` — the formats `export` can write.

Every unsupported route answers with `capability/not-implemented`.

## Support matrix

| Area | Status | Notes |
| :--- | :--- | :--- |
| `.ocad` project JSON | partial | Format version 1, create/open/save/reopen. The same file is the CLI's unit of work. |
| DXF | partial | Declared ASCII import subset plus `dxf` and `dxf-r12` export. |
| SVG | supported | Preview renderer, export target and a declared import subset. |
| PDF | supported | Export target only; not a renderer and not editable. |
| STEP | partial | Export direction only (AP203 solids and faces). |
| OBJ | partial | Export direction only (triangle meshes). |
| STL | export target only | Recorded as `unsupported` under `formats` because there is no STL reader; `exportTargets.stl` is `supported`. |
| IGES/IGS | partial | Export direction only (a declared IGES-5.3 deck subset); no reader. |
| DWG, PNG, JPG | unsupported | No reader and no writer. |
| Interactive viewport | unsupported | SVG output is the only renderer; the studio does not paint geometry. |
| Constraint solver, 3D kernel | unsupported | No constraints, booleans, analytic surfaces or solids beyond faceted projection. |

Entity kinds reported by `entities`: `line`, `circle`, `arc`, `ellipse`,
`point`, `polyline`, `spline`, `text`, `dimension`, `radial-dimension`,
`ordinate-dimension`, `angular-dimension`, `hatch`.

Operations reported as supported: `transform`, `copy`, `array`, `explode`,
`offset`, `trim`, `extend`, `break`, `join`, `fillet`, `chamfer`, `align`,
`distribute`, `stretch`, `overkill`, `hatch-pattern`, `title-block`,
`polyline-smooth`, `polyline-chamfer`, `diff`, `quantity-table`, `measure`,
`measure-entity`, `measure-points`, `stats`,
`dimension-style`, `text-style`, `text-search`, `group`, `select`, `properties`,
`snap`, `linetype`, `ucs`.
`constraints` and `kernel3d` are reported as unsupported.

## DXF import subset

The reader accepts ASCII group-code pairs. It reads `$INSUNITS`, the LAYER
table, BLOCKS definitions, and `LINE`, `CIRCLE`, `ARC`, `ELLIPSE`, `POINT`,
`TEXT`, `LWPOLYLINE` and the older `POLYLINE`/`VERTEX`/`SEQEND` sequence,
including `INSERT` expanded into the current space at zero rotation and uniform
scale, and the `ATTRIB` values that follow an insert.

- `ARC` and `ELLIPSE` stay analytic. A rotated or aligned linear `DIMENSION`,
  and radius, diameter and three-point angular dimensions, become the matching
  dimension entity.
- `SPLINE` becomes a cubic spline through its fit points; if it has none, through
  its control polygon. The curve is approximated with uniform Catmull-Rom spans;
  the DXF control points and knot vector are not kept.
- `LEADER` becomes an open polyline. Arrowheads and annotation text are not
  modelled.
- A 3D polyline is projected onto the XY plane and loses Z.
- A `HATCH` becomes a closed filled polygon with its pattern name and one
  boundary path. A hatch with a curved boundary edge is skipped.
- Polyline bulge arcs are flattened to line segments.
- A layer colour is read from code `62` and mapped through the ACI palette; a
  `420` true-colour value takes precedence when both are present.
- Unitless files assume millimetres and report it.

Losses are returned in `losses` as `{ code, severity, detail }`. `notice` means
geometry was kept with a documented approximation. `warning` means content was
skipped or replaced and the imported drawing differs from the source. Nothing is
dropped silently.

| Code | Severity | Meaning |
| :--- | :--- | :--- |
| `dxf/assumed-units` | notice | No `$INSUNITS`; millimetres assumed. |
| `dxf/layer-color` | warning | A layer colour was not a valid `#rrggbb` value and was written as ACI 7. |
| `dxf/implicit-layer` | warning | An entity referenced an undeclared layer and a layer was created. |
| `dxf/expanded-block` | notice | An `INSERT` was expanded into the current space. |
| `dxf/block-recursion` | warning | Recursive block nesting stopped at depth 8. |
| `dxf/missing-block` | warning | An `INSERT` referenced a block that is not defined and was skipped. |
| `dxf/unsupported-entity` | warning | One message per entity type the reader does not implement; includes the first handle and line. |
| `dxf/dimension-type` | notice | A dimension type outside the subset was converted or skipped. |
| `dxf/dimension-offset` | notice | A dimension offset was approximated. |
| `dxf/attribute-text` | notice | Block attribute values became text. |
| `dxf/attribute-empty` | warning | An empty attribute value was skipped. |
| `dxf/spline-fit-points` | notice | `SPLINE` became a Catmull-Rom cubic spline through its fit points; control points and knot vector not kept. |
| `dxf/spline-control-polygon` | notice | `SPLINE` had no fit points and became a Catmull-Rom cubic spline through its control polygon. |
| `dxf/leader-polyline` | notice | `LEADER` imported as an open polyline; arrows and text dropped. |
| `dxf/projected-3d` | notice | 3D polyline projected to XY; Z dropped. |
| `dxf/hatch-boundary` | notice | Hatch imported as a closed filled polygon. |
| `dxf/hatch-bulge` | notice | Hatch boundary bulge arcs flattened. |

Not read: `DWG`, `MLEADER` geometry, `MLINE`, `3DSOLID`, `REGION`, `WIPEOUT`,
`IMAGE`, `XREF`, proxy entities, and every other entity type outside the list
above. A two-line angular dimension (DXF type 2) is skipped.

## SVG import subset

The reader accepts an `<svg>` document and keeps the geometry elements `line`,
`circle`, `ellipse`, `rect`, `polyline`, `polygon`, `path` and `text` on the
default layer. Coordinates are read as authored in user units.

- `line`, `circle` and `ellipse` stay analytic; an `ellipse` is imported as a
  full ellipse.
- `rect` becomes a closed polyline; `polyline` and `polygon` become open and
  closed polylines.
- Path moves, lines and closes (`M`, `L`, `H`, `V`, `Z` and their relative
  forms) become polylines. Cubic and quadratic commands (`C`, `S`, `Q`, `T`)
  are sampled into sixteen segments per span. An elliptical arc (`A`) or any
  other command truncates the path at that command.
- `text` becomes a text entity at its `x`/`y` position with the `font-size`
  height; inner markup such as `tspan` is flattened into the string.
- Definition subtrees (`defs`, `symbol`, `marker`, `pattern`, `filter`, `mask`,
  `clipPath`, `style`, `script`, `title` and `desc`) are skipped.
- `transform` attributes, nested viewports and presentation styling are not
  applied; the SVG y-axis is kept as authored.

Losses use the same `{ code, severity, detail }` shape as DXF import.

| Code | Severity | Meaning |
| :--- | :--- | :--- |
| `svg/assumed-units` | notice | SVG carries no unit metadata; user units were read as millimetres. |
| `svg/transform` | warning | A `transform` attribute was ignored; geometry was read as authored. |
| `svg/path-curve-flattened` | notice | Curve commands were sampled into line segments. |
| `svg/path-unsupported-command` | warning | A path was truncated at an unsupported command such as `A`. |
| `svg/rect-corner` | notice | A rounded rectangle was written with sharp corners. |
| `svg/element-invalid` | warning | An element was skipped for missing or non-finite geometry. |
| `svg/element-unsupported` | warning | One message per element tag the reader does not implement. |

Not read: raster content, text styling, animation, nested viewports and every
element outside the list above.

## DXF export

`dxf` writes R12-style ASCII and declares `AC1015` when the drawing has custom
line types, lineweights, a hatch, a spline, or a non-plottable layer; otherwise
it declares `AC1009`. It writes `$INSUNITS`, the LTYPE, LAYER, STYLE and DIMSTYLE
tables, LINE, CIRCLE, ARC, ELLIPSE, POINT, TEXT, LWPOLYLINE, SPLINE,
linear/radial/ordinate/angular DIMENSION, and a HATCH that uses its boundary
polygon with the pattern name only. `dxf-r12` always declares `AC1009` and always
reports omissions.

Each layer colour is approximated by the nearest of nine standard ACI indices
(1 red, 2 yellow, 3 green, 4 cyan, 5 blue, 6 magenta, 7 white, 8 grey, 9 light
grey) and written on code `62`. The mapping round-trips at the palette values and
is only a warning for a value that is not valid `#rrggbb`, which is written as
ACI 7.

Reported losses: `dxf/r12-units`, `dxf/r12-lineweight`, `dxf/r12-plottable`,
`dxf/r12-ellipse`, `dxf/r12-spline`, `dxf/dimension-lines`,
`dxf/hatch-boundary`, `dxf/r12-hatch`, `dxf/hatch-pattern`, and
`dxf/layer-color` only for an invalid colour. A hatch entity writes its pattern
name only, never its pattern geometry; line entities created by `hatchPattern`
are ordinary `LINE` records and are written in full.

Not written: `DWG`, blocks, `XREF`, images, layouts and paper space, proprietary
or proxy objects, text styles beyond the declared table fields, and any
3D content.

## IGES export

`iges` writes an IGES-5.3 ASCII deck (80-column cards, `S`, `G`, `D`, `P` and
`T` sections) containing the drawing's curve entities in the XY plane. The deck
is deterministic: the same drawing always produces the same bytes. Parameter
data fills columns 1-72 and the last parameter card of every entity carries a
negative sequence number, the directory structure lines mark each entity as the
root of its own structure, and the terminate card reports the size of every
section and the card number where each section starts. The global
section declares the producer, the IGES version, the card width and the units
taken from the drawing (`MILLIMETE` or `INCH`); the deck carries curve entities
only, with no transform, section or property definition entities.

| Entity | IGES type |
| :--- | :--- |
| `LINE` | 110 (straight segment) |
| `POINT` | 100 (general point) |
| `CIRCLE` | 112 (circle) |
| `ARC` (full sweep) | 112 (circle) |
| `ARC` (partial) | 126 exact rational quadratic B-spline, split at 90° turns |
| `SPLINE` | 126 cubic B-spline through the Catmull-Rom fit points |
| `ELLIPSE` (full) | 114 (ellipse) |
| `ELLIPSE` (partial) | 110 segments sampled from the arc |
| `POLYLINE` | one 110 per segment; a closed polyline repeats the first point |

Text, every dimension kind and hatch are annotation entities and are skipped.
Layer colours, lineweights and line types have no IGES counterpart in this
subset and are dropped. The export report carries the written entity count in
`entities`.

| Code | Severity | Meaning |
| :--- | :--- | :--- |
| `iges/unsupported-entity` | warning | One message for the total count of text, dimension and hatch entities skipped. |
| `iges/ellipse-polyline` | warning | One message for the total count of partial ellipses flattened to line segments. |
| `iges/layer-info` | warning | The drawing carries layers beyond the built-in pair; their names, colours and line types are not written. |

There is no IGES reader.

## SVG and PDF

Both formats skip hidden, frozen and non-plottable layers. They stroke every
visible layer at its lineweight — zero lineweight uses the SVG/PDF default — and
reduce the line type to one dash/gap pair so a dashed layer reads as dashed. They
draw hatch boundary polygons, not the pattern definition; the line entities
`hatchPattern` creates are ordinary visible entities and are drawn normally.

Both formats draw the same engineering sheet: two borders inset from the sheet
edge and a title block in the lower-right corner of the inner border carrying
the drawing title, the author/company/project byline, the declared units and the
current revision. SVG is the only renderer. PDF is A4 landscape with one
uniform fit into the area left by the inner border and the title block, so
geometry never crosses either of them. After fitting, PDF keeps every text
entity — notes and title-block text included, not only dimension labels — at
the 7 pt legible print floor. A linear dimension whose span cannot hold two
arrowheads and the label draws both arrowheads outside the extension lines
pointing inward, so the label never sits inside crossing arrows. PDF
approximates arcs and ellipses with cubic Bezier segments and reports
`pdf/text-encoding` when a character cannot be
encoded, including characters in the title block. PDF is not editable:
reopening it is not supported. SVG can be reopened through the declared import
subset above.

## STL, OBJ and STEP

All three are export-only projections in the XY plane, extruded along +Z with
thickness `1.0`, arcs and ellipses sampled at five-degree steps, and polygons
filled by ear clipping. They are faceted approximations, not analytic solids;
there are no booleans, curved surfaces or units beyond the drawing's declared
unit.

- `stl` writes one binary STL solid with one-based triangle ids.
- `obj` writes the same triangles as `v` and `f` records with no materials or
  normals.
- `step` writes AP203 ISO-10303-21 with each triangle as an `ADVANCED_FACE`; a
  closed profile also gets a `CLOSED_SHELL` and `MANIFOLD_SOLID_BREP`.

Unsupported geometry is skipped and reported: `stl/open-profile`,
`obj/open-profile`, `step/open-profile` (an open or degenerate profile),
`stl/invalid-profile`, `obj/invalid-profile`, `step/invalid-profile`, and
`stl/degenerate-triangle`, `obj/degenerate-triangle`,
`step/degenerate-triangle`.

## Inspection and measurement

`inspect` runs `units`, `layers`, `geometry`, `polylines`, `dimensions`,
`duplicate-entity`, `zero-length` and `open-contour` by default; `empty-layer`
is opt-in. A failing inspection exits `1` from the CLI and returns
`inspection.failed === true` from the SDK.

Measurement samples arcs and ellipses at five-degree steps. Bounds are computed
from the sampled outline, not the exact curve. Perimeter and area use the
sampled polygon, so a drawing with curved entities reports approximations.
`measure --sheet` filters by the named layout; drawing-wide totals are only
available without a sheet name.

`stats` reports one entry per layer in `layers`, plus a `groups` list that
follows `groupBy`. `layer` keeps the layer order and includes layers that carry
no entity; `type`, `color` and `linetype` list only keys that occur, sorted by
key, so the result does not depend on insertion order. Length and area use the
same rules as `measure`, so a full circle contributes area but no length.

## Object snap

`snap` computes candidates without changing the drawing. Modes are `endpoint`,
`midpoint`, `center`, `quadrant`, `nearest`, `intersection`, `perpendicular` and
`tangent`. A request that names no modes keeps the original set —
`endpoint`, `midpoint`, `center` and `intersection` — so `quadrant`, `nearest`,
`perpendicular` and `tangent` are opt-in.

- `center` covers circles, arcs and ellipses.
- `quadrant` covers circles and arcs at 0, 90, 180 and 270 degrees. An arc only
  reports the quadrants inside its counter-clockwise sweep.
- `nearest` projects the probe point onto a line, one polyline segment, a circle
  or an arc. It needs a probe point; an arc result is clamped to the start and
  end angles.
- `perpendicular` needs a probe point too. On a line or one polyline segment it
  reports the foot of the perpendicular only when that foot lies on the bounded
  segment, including the closing edge of a closed polyline. On a circle it
  reports both points where the line through the centre and the probe meets the
  circle, and on an arc only the points inside its counter-clockwise sweep. A
  probe at a circle or arc centre yields no candidate. Ellipses, splines, points,
  text, hatches and dimensions contribute none.
- `tangent` needs a probe point too. On a circle whose outside holds the probe it
  reports the two contact points of the tangents from the probe, at the
  centre-to-probe direction rotated by plus and minus `acos(r / d)`. A probe on
  the circle reports that point, and a probe inside the circle or at its centre
  yields no candidate. On an arc only the contact points inside its
  counter-clockwise sweep remain. Lines, polylines, ellipses, splines, points,
  text, hatches and dimensions contribute none.
- `intersection` covers bounded line-to-line, line-to-circle, circle-to-circle,
  line-to-arc, arc-to-circle and arc-to-arc pairs. A crossing with an arc must
  lie on the arc's counter-clockwise sweep. Tangency, parallel lines, concentric
  circles and separated pairs produce no candidate.

Candidates sort by distance to the probe point, then by entity id, then by mode,
so equal-distance results are deterministic. A tolerance keeps only candidates
within that distance and needs a probe point. A degenerate circle with radius 0
contributes no quadrant, nearest, perpendicular, tangent or intersection
candidate, and polyline nearest and perpendicular points are computed segment by
segment, including the closing edge of a closed polyline. Ellipses and splines
offer `endpoint`, `midpoint` and `center` only; quadrants, nearest and
perpendicular points are analytic for lines, polylines, circles and arcs, and
tangent points for circles and arcs.

## Offset

Offsetting has two forms. `offset(entityId, distance)` keeps the original
signed convention: a positive `distance` grows a circle or arc and offsets a
line or open polyline to the left of its travel direction, while a negative
distance shrinks or offsets to the right. A zero or non-finite distance reports
`session/argument-invalid`. `offsetBySide(entityId, distance, side)` requires a
positive finite distance and names the direction explicitly:

- `left` and `right` apply to a line and polyline, measured along the travel
  direction.
- `outside` and `inside` apply to a circle and arc, where only the radius
  changes. A circle keeps its centre; an arc also keeps its centre and start
  and end angles.

In both forms a line becomes an equal-length parallel line, and a polyline
offsets every segment by the same distance and joins consecutive segments with
a miter. The new entity takes the next deterministic `entity-N` id, inherits the
source layer — and therefore its colour, line type and lineweight — and
increments the revision.

With `offsetBySide`, a side that does not match the entity kind reports
`offset/invalid-side`; a resulting radius at or below zero or a non-finite
result reports `offset/invalid-distance`; an unknown id reports
`offset/not-found`; an unsupported kind reports `offset/unsupported-entity`. The
signed form keeps its original `session/argument-invalid` and
`object/unsupported` codes.

A polyline corner whose miter would exceed four times `distance` is refused with
`offset/miter-limit` by `offsetBySide` instead of being silently bevelled. The
signed form keeps its original miter fallback for nearly collinear normals. In
both forms the offset is not clipped against itself, so a closed polyline whose
new edge crosses another edge is returned as computed. Polylines hold straight
segments only, so there are no arc or bulge segments to miter. Ellipse, spline,
point, text, hatch and dimension entities stay unsupported.

## Trim

`trim(id, cutter)` cuts one line, arc or circle target at a line, circle or arc
cutter and updates that target in place. All intersections are bounded: a line
cutter must cross the target segment, a circle or arc crossing must lie on the
arc's counter-clockwise sweep, and a crossing of an arc target must lie on the
target's own sweep. Of the two sides a single crossing produces, the longer one
is kept. When the cutter crosses twice, the longer of the two outer segments is
kept, so the span between the cut points and the shorter outer piece are
discarded. Equal lengths keep the side that starts at the target's original
`from` endpoint. An arc target follows the same policy, measured in degrees
travelled from its start angle, and stays an arc with the same centre and
radius.

A circle has no endpoints, so it needs exactly two distinct crossings: the
shorter span between them is removed and the target becomes an arc covering the
longer sweep. The response carries `{ revision, drawing, entity, cutter }`, where
`entity` is the updated target, possibly an arc that came from a circle. The
cutter never moves, only the target's layer is checked for locks, and the edit
is one history step.

A cutter that never meets the target reports `trim/no-intersection`. Tangency,
collinearity, a zero-length or zero-radius input, a single or ambiguous circle
cut, and a crossing that falls on a target endpoint (so nothing would be
removed) report `trim/degenerate`. A target that is not a line, arc or circle,
or a cutter that is not a line, circle or arc, reports `object/unsupported`; an
unknown id reports `object/not-found`, a locked target layer reports
`drawing/layer-locked`, and the same id for target and cutter reports
`session/argument-invalid`.

## Extend

`extend(id, boundary, side)` lengthens one line or arc until it meets a line,
circle or arc boundary. `side` is `end` by default: for a line the `from`
endpoint stays put and `to` moves along the `from`-to-`to` direction, while
`side = start` keeps `to` and moves `from` along the `to`-to-`from` direction.
Only crossings ahead of the moving endpoint count, and the nearest one becomes
the new endpoint, so the line grows and never folds back. A line boundary is its
bounded drawn segment; a circle or arc crossing must lie on the arc's
counter-clockwise sweep.

An arc target travels along its own circle instead of a tangent ray: `end` walks
counter-clockwise from the end angle and `start` walks clockwise from the start
angle, and the first crossing with the boundary becomes the new endpoint. The
centre, radius and other endpoint stay put. A crossing at the moving endpoint is
ignored, and a candidate that would push the sweep to a full turn is refused
rather than wrapping the arc.

The response carries `{ revision, drawing, entity, boundary, side }`, where
`entity` is the extended line or arc. The boundary never moves, only the target's
layer is checked for locks, and the edit is one history step.

A boundary the extension never reaches (including one that only crosses behind
the endpoint or past a full turn) reports `extend/no-intersection`. A target
that is not a line or arc, or a boundary that is not a line, circle or arc,
reports `object/unsupported`; an unknown id reports `object/not-found`, a
locked target layer reports `drawing/layer-locked`, and a zero-length line, a
zero-radius arc or the same id for target and boundary reports
`session/argument-invalid`.

## Break

`breakEntity(id, at)` splits one line or arc at `at`, and the edit is one
history step and one revision.

A line point must lie on the segment and strictly between its endpoints: the
tolerance is `1e-9` scaled by the segment length, and a point inside that band
but off the line is projected onto it. The two pieces therefore stay collinear
and their lengths add up to the original. The source is replaced in place by two
lines with fresh deterministic ids.

An arc point must lie on the arc's circle within `1e-9` scaled by the radius, and
its angle must fall inside the counter-clockwise sweep away from both endpoints,
using the same degree tolerance as `trim`. The source keeps the part from its
start angle to the cut angle, and a new arc with the next deterministic id covers
the cut angle to the original end angle. Both share the centre, radius and layer.

The response carries `{ revision, drawing, removed, created }`, where `removed`
is the source id and `created` holds the two pieces in order. For a line, the
pieces run from the original `from` endpoint to the break point and from the
break point to the original `to` endpoint, both with fresh ids and on the source
layer. For an arc, the first piece reuses the source id. The source line is
removed from draw order and entity groups; the appended arc keeps its place.

A point on an endpoint or off the entity, a zero-length line and a zero-radius
arc report `break/not-on-entity`. A circle or another kind reports
`object/unsupported`; an unknown id reports `object/not-found`; a locked or
frozen layer reports `drawing/layer-locked` or `drawing/layer-frozen`; non-finite
coordinates report `session/argument-invalid`.

| Code | Cause |
| :--- | :--- |
| `break/not-on-entity` | The point is on an endpoint, off the segment or arc, or the line has zero length or the arc zero radius. |
| `object/unsupported` | The target is not a line or arc. |
| `object/not-found` | The id does not name an entity. |
| `drawing/layer-locked` | The entity's layer is locked. |
| `drawing/layer-frozen` | The entity's layer is frozen. |
| `session/argument-invalid` | The coordinates are not finite. |

The model tests cover the line midpoint split (`break_line_at_midpoint_creates_two_collinear_lines`),
the arc split with exact angles (`break_arc_at_interior_point_splits_into_two_arcs`),
the wrapped arc sweep (`break_arc_keeps_the_wrapped_sweep_convention`),
rejected points (`break_rejects_endpoints_outside_points_and_off_line_points` and
`break_arc_rejects_endpoints_and_points_off_the_arc`),
non-line, zero-radius and locked-layer refusals
(`break_rejects_non_line_entities_and_locked_layers` and
`break_arc_rejects_zero_radius_and_locked_layers`),
and repeatability (`break_is_deterministic`).

## Join

`joinEntities(a, b)` merges two collinear lines that share one endpoint into a
single line. Both entities must be lines on the same layer, so colour, line type
and lineweight stay consistent. One endpoint of `a` must coincide with one
endpoint of `b` within a tolerance of `1e-9` scaled by the longer segment length
(and at least 1), and the two lines must be collinear within the same tolerance
and continue in opposite directions from that shared point, so the union is one
longer segment rather than an overlap.

The two sources are removed and replaced, at `a`'s list position, by one line
from `a`'s free endpoint to `b`'s free endpoint. The merged line keeps the shared
layer and receives a fresh deterministic id (`entity-N`); both sources leave
draw order and entity groups. The edit is one history step and one revision.

The response carries `{ revision, drawing, removed, created }`, where `removed`
holds both source ids in `[a, b]` order and `created` holds the merged line.

Endpoints that do not meet, lines that are not collinear, and a folded-back
overlap where both free endpoints lie on the same side of the shared point report
`join/not-adjacent`. A non-line or a pair on different layers reports
`object/unsupported`; an unknown id reports `object/not-found`; a locked or
frozen layer reports `drawing/layer-locked` or `drawing/layer-frozen`; the same
id twice or a zero-length line reports `session/argument-invalid`.

| Code | Cause |
| :--- | :--- |
| `join/not-adjacent` | The endpoints do not meet, the lines are not collinear, or they fold back over each other. |
| `object/unsupported` | An entity is not a line, or the two lines are on different layers. |
| `object/not-found` | An id does not name an entity. |
| `drawing/layer-locked` | The shared layer is locked. |
| `drawing/layer-frozen` | The shared layer is frozen. |
| `session/argument-invalid` | Both ids are the same, or a line has zero length. |

The model tests cover horizontal and slanted merges
(`join_horizontal_lines_into_one_segment`, `join_slanted_lines_keeps_the_free_endpoints`),
reversed endpoints (`join_accepts_a_reversed_second_line`), rejected pairs
(`join_rejects_lines_that_do_not_meet_or_are_not_collinear`), non-line and
cross-layer refusals (`join_rejects_non_lines_and_cross_layer_pairs`), locked
layers (`join_refuses_a_locked_layer`) and repeatability (`join_is_deterministic`).

## Align and distribute

`align(request)` moves a selection of entities onto one bounding-box edge or
centreline. Each entity's box comes from the same sampled outline `measure`
reports. `left`, `right`, `bottom` and `top` pull the extreme edges of every
box onto the smallest or largest edge of the selection; `center` and `middle`
pull the horizontal or vertical box centres onto the midpoint of the
selection's span, so the widest and tallest boxes stay put. Entities move by
translation only; ids, layers and geometry are otherwise untouched.

`distribute(request)` spreads a selection along `x` or `y` with equal gaps
between consecutive bounding boxes. Entities are ordered by box centre along
the axis, ties keeping document order; the first and last entities stay put and
every interior entity is translated so the gap matches
`(span - total size) / (count - 1)`. The gap may be negative when the boxes
overlap, which still yields a deterministic equal-spacing layout.

The align response carries `{ revision, drawing, mode, affected, bounds }` and
the distribute response carries `{ revision, drawing, axis, affected, bounds }`
with `bounds` covering the whole drawing after the edit. Each call is one
history step and one revision; repeated ids are applied once.

| Code | Cause |
| :--- | :--- |
| `align/insufficient-entities` | Fewer than two distinct entity ids after deduplication. |
| `distribute/insufficient-entities` | Fewer than three distinct entity ids after deduplication. |
| `object/not-found` | An id does not name an entity. |
| `object/degenerate` | An entity has no finite bounds. |
| `drawing/layer-locked` | A selected entity sits on a locked layer. |
| `drawing/layer-frozen` | A selected entity sits on a frozen layer. |
| `session/argument-invalid` | The id list is empty, or the mode/axis string is not one of the accepted names. |

The model tests cover every mode (`align_left_edges_share_the_smallest_min_x`,
`align_center_and_middle_use_the_selection_midpoints`,
`align_top_and_bottom_edges_share_the_extremes`), the insufficient-entity,
unknown-id and empty-list refusals
(`align_rejects_fewer_than_two_entities`,
`distribute_rejects_fewer_than_three_entities`), locked, frozen and degenerate
refusals (`align_refuses_locked_and_frozen_layers_and_degenerate_entities`,
`distribute_refuses_locked_layers_and_degenerate_entities`), the gap layout
(`distribute_equalizes_gaps_along_x`, `distribute_along_y_orders_by_box_center`),
single history steps with undo/redo
(`align_and_distribute_are_single_history_steps`), the save/reopen round trip
(`align_and_distribute_survive_save_and_reopen`) and repeatability
(`align_is_deterministic`).

## Stretch

`stretch(request)` moves the defining points that lie inside a crossing window
(boundary included) by a displacement. The rectangle is normalised like `select`
and tested with crossing semantics, so an entity joins the edit when any sampled
outline point is inside it or its outline crosses it. Each supported kind maps
one-to-one to the model:

- `line`: `from` and `to`;
- `polyline`, `spline` and `hatch`: their ordered points;
- `point`: its position;
- `circle`, `arc` and `ellipse`: their centre, the same point `transform` and
  `entity move` translate.

Text and every dimension kind have no stretchable vertices; they are skipped
and counted in `skipped`. A line or curve that crosses the window without any
defining point inside is left unchanged and is not counted as affected. Ids,
layers and draw order are untouched. Every accepted request writes one revision
and one history step, including a request that matches nothing (`affected = 0`).
The response carries `{ revision, drawing, window, displacement, affected,
skipped }`.

| Code | Cause |
| :--- | :--- |
| `session/argument-invalid` | A window corner or displacement value is not finite, the window has zero width and height, or no `dx`/`dy` displacement is supplied. |
| `drawing/layer-locked` | An entity that would move sits on a locked layer. |
| `drawing/layer-frozen` | An entity that would move sits on a frozen layer. |

The model tests cover partial line and polyline movement, boundary-inclusive
vertices, curve centres with their shape preserved, spline and hatch points,
annotation skipping with identity/order/layer preservation, single revision and
history steps with undo/redo, the empty selection revision, and invalid windows,
displacements, locked and frozen layers.

## Hatch patterns

`hatchPattern(request)` fills one `hatch` entity with real `line` entities
clipped to its boundary. The hatch keeps its boundary polygon and pattern name;
the generated lines sit on the hatch's layer, so every exporter carries the
fill. The scanline algorithm rotates the boundary by `-angle` so the pattern
lines become horizontal, crosses the rotated edges under a half-open rule that
counts a shared vertex once, sorts the crossings with a total order and pairs
them by the even-odd rule. Crossings are anchored to the world origin at
`v = k * spacing`, and levels within `1e-9` of the boundary are skipped so the
boundary is not traced a second time and a cardinal angle's rotation error
cannot add a coincident line. `cross` adds a second family at `angle + 90`.

`angle` defaults to `0`, `spacing` to `5` and `cross` to `false`. The
response carries `{ revision, drawing, created }`, where `created` holds the new
line entities in deterministic id order. The edit is one history step and one
revision, and a request that adds no line (a boundary thinner than `spacing`)
still increments the revision.

| Code | Cause |
| :--- | :--- |
| `hatch/invalid-angle` | `angle` is not finite. |
| `hatch/invalid-spacing` | `spacing` is not positive and finite. |
| `hatch/spacing-too-fine` | One family would need more than 100,000 scanlines. |
| `hatch/unsupported-entity` | The id names an entity that is not a hatch. |
| `hatch/solid-pattern` | The hatch is a `SOLID` fill with no pattern to expand. |
| `hatch/degenerate-boundary` | The boundary has zero area. |
| `object/not-found` | The id does not name an entity. |
| `drawing/layer-locked` | The hatch layer is locked. |
| `drawing/layer-frozen` | The hatch layer is frozen. |

The model tests cover the default fill and layer inheritance
(`hatch_pattern_fills_a_closed_boundary_with_lines`), explicit angle, spacing
and `cross` (`hatch_pattern_honours_angle_spacing_and_cross`), and every refusal
including locked and frozen layers
(`hatch_pattern_rejects_bad_requests_and_protected_layers`).

## Title block

`titleBlock(request)` draws a sheet frame and a title block on one layer. The
sheet is a rectangle at the origin (`width` × `height`, A3 landscape by
default); the operation writes an outer border, an inner frame inset by
`margin`, a block frame in the bottom-right corner and one row per field:
Title, Number, Revision, Scale, Date, Author and Sheet. A label column and one
value column sit inside the block. Values default to the drawing title, id,
revision, `1:1`, an empty date, the drawing author and `1`; an override wins and
an empty override renders a blank cell, so only the grid remains. Everything is
a real `line` or `text` entity with a fresh deterministic id on `layer`
(default `titleblock`, created with a neutral colour when missing).

`width`, `height`, `margin` and `textHeight` default to the A3 landscape size,
`10` and `3.5`. The response carries `{ revision, drawing, created }`. The edit
is one history step and one revision.

| Code | Cause |
| :--- | :--- |
| `session/argument-invalid` | A dimension is not finite or not positive, or an override exceeds the property limit. |
| `titleblock/invalid-sheet` | The sheet is too small for the frame and block. |
| `drawing/layer-locked` | The target layer is locked. |
| `drawing/layer-frozen` | The target layer is frozen. |

The model tests cover the default A3 sheet, layer creation and default values
(`title_block_generates_a_framed_sheet_with_defaults`), custom dimensions and
overrides on a custom layer (`title_block_honours_overrides_and_a_custom_sheet`)
and the invalid-dimension, invalid-sheet, long-override, locked and frozen
refusals (`title_block_rejects_bad_sheets_and_protected_layers`).

## Quantity take-off

`quantityTable(request)` draws a framed quantity take-off table from the same
aggregates `stats` reports. One row per group of `groupBy` (`layer` by default,
or `type`, `color`, `linetype`) carries the entity count, path length and
enclosed area, and a total row closes the table. A caption row sits above the
header unless `title` is empty. The bottom-left corner is `origin` (default the
origin), the table is `60 * textHeight` wide and every row is `2.5 * textHeight`
tall; `textHeight` defaults to `3.5`. Columns are Group, Count, Length and Area,
and the values use at most three decimals without trailing zeros. Rows are
computed before the table is inserted, so a layer grouping never lists the
table's own entities. Everything is a real `line` or `text` entity with a fresh
deterministic id on `layer` (default `quantity`, created with a neutral colour
when missing). The response carries `{ revision, drawing, groupBy, columns,
rows, totals, created }`, and the edit is one history step and one revision.

| Code | Cause |
| :--- | :--- |
| `session/argument-invalid` | `textHeight` is not positive and finite, `origin` is not finite, or the caption exceeds the property limit. |
| `drawing/layer-locked` | The target layer is locked. |
| `drawing/layer-frozen` | The target layer is frozen. |

The model tests cover the framed table, its totals and its geometry
(`quantity_table_draws_a_framed_table_with_totals`), a custom origin, caption,
text height and type grouping
(`quantity_table_honours_origin_caption_and_type_grouping`), the invalid-input,
locked and frozen refusals
(`quantity_table_rejects_bad_requests_and_protected_layers`) and repeatability
(`quantity_table_is_deterministic`). Grouping itself is covered by
`stats_groups_entities_by_type_colour_and_linetype`.

## Polyline smoothing

`smoothPolyline(request)` rounds the corners of one `polyline` with Chaikin
corner cutting. Each pass replaces every segment by the point at `ratio` and
the point at `1 - ratio` along it, so an open chain keeps its first and last
vertex and a closed chain keeps its closed flag and never duplicates the first
vertex. The entity keeps its id, layer and draw-order position, and the edit is
one history step and one revision.

`iterations` defaults to `2` and must be within `1..=8`; `ratio` defaults to
`0.25` and must be strictly between `0` and `0.5`. The response carries
`{ revision, drawing, entity }`.

| Code | Cause |
| :--- | :--- |
| `polyline/unsupported-entity` | The id names an entity that is not a polyline. |
| `polyline/degenerate` | The chain has fewer than three vertices. |
| `polyline/invalid-iterations` | `iterations` is outside `1..=8`. |
| `polyline/invalid-ratio` | `ratio` is not finite or outside the open interval `(0, 0.5)`. |
| `polyline/vertex-limit` | A pass would exceed 100,000 vertices. |
| `object/not-found` | The id does not name an entity. |
| `drawing/layer-locked` | The polyline layer is locked. |
| `drawing/layer-frozen` | The polyline layer is frozen. |

The model tests cover the open chain (`smooth_polyline_rounds_an_open_chain_and_keeps_its_identity`),
the closed chain without a duplicated vertex
(`smooth_polyline_keeps_a_closed_chain_closed_without_duplicating_the_first_vertex`),
every refusal (`smooth_polyline_rejects_bad_requests_and_protected_layers`) and
the single undoable history step (`smooth_polyline_is_one_undoable_history_step`).

## Polyline chamfer

`polylineChamfer(request)` bevels every corner of one polyline. Every interior
vertex of an open chain, and every vertex of a closed chain including the
first, is trimmed `distance` along both adjacent segments. The corner point is
removed and the two trim points take its place, so the entity stays one
polyline with two extra vertices per beveled corner. It keeps its id, layer and
closed flag, and the edit is one history step and one revision.

`distance` must be positive and finite. Every adjacent segment must be longer
than `2 * distance`; a segment shorter than or equal to that cannot supply both
trim points, so the whole edit is refused with `session/argument-invalid` and
nothing is written. An open chain keeps its first and last vertex unchanged.
Polylines hold straight segments only, so there is no bulge or arc segment to
preserve. The response carries `{ revision, drawing, entity }`.

| Code | Cause |
| :--- | :--- |
| `session/argument-invalid` | `distance` is not positive and finite, or a segment is not longer than `2 * distance`. |
| `polyline/unsupported-entity` | The id names an entity that is not a polyline. |
| `polyline/degenerate` | The chain has fewer than three vertices, so it has no corner to bevel. |
| `object/not-found` | The id does not name an entity. |
| `drawing/layer-locked` | The polyline layer is locked. |
| `drawing/layer-frozen` | The polyline layer is frozen. |

The model tests cover the open chain (`polyline_chamfer_bevels_every_interior_vertex_of_an_open_chain`),
the closed chain including its first corner
(`polyline_chamfer_bevels_a_closed_chain_including_the_first_corner`), every
refusal without a write (`polyline_chamfer_rejects_short_segments_and_bad_requests_without_writing`)
and the single undoable step with the save/reopen round trip
(`polyline_chamfer_is_one_undoable_step_and_survives_a_round_trip`).

## Ellipse and spline editing

`updateEllipse` and `updateSpline` edit one existing entity in place. The entity
keeps its id and layer, the edit is one history step and the revision bumps
exactly once.

`updateEllipse` changes only the supplied fields of one `ellipse`: `rx`, `ry`,
`rotation`, `startAngle` and `endAngle`. At least one field is required and every
omitted field keeps its current value, so a full ellipse stays full unless the
start or end parameter changes. `rx` and `ry` must be positive and finite and the
angles finite; a full sweep is a start equal to the end or the default
`0`-`360` pair, and the sweep always runs counter-clockwise from the start.

`updateSpline` mirrors the polyline edit on the spline's fit points: a
replacement `points` list, one `insertAt`/`insertPoint` pair, one `removeAt`
index, one `moveAt`/`moveTo` pair or a new `closed` flag. Replacing the list
cannot be combined with a single fit-point edit, one call edits at most one fit
point, and `insertAt` accepts the current fit-point count to append. A spline
always keeps at least two fit points, so a replacement list with fewer than two
or a removal that would leave fewer is refused. The curve is the Catmull-Rom
spline through its fit points, so inserting, moving or removing a fit point
changes the drawn curve accordingly.

Both operations report:

| Code | Cause |
| :--- | :--- |
| `session/argument-invalid` | No field is supplied, `rx`/`ry` is not positive and finite, an angle is not finite, an index is outside the list, the insert and point (or move and point) arguments are unpaired, a replacement list has fewer than two fit points, or a removal would leave fewer. |
| `object/unsupported` | The id names an entity that is not an ellipse (or not a spline). |
| `object/not-found` | The id does not name an entity. |
| `drawing/layer-locked` | The entity's layer is locked. |
| `drawing/layer-frozen` | The entity's layer is frozen. |

The model tests cover every ellipse field with untouched fields keeping their
values, the spline closed flag, replacement lists, insertion, removal and moving
at the boundaries, both wrong-kind refusals, unknown ids, locked and frozen
layers, the single revision bump, the save/reopen round trip and determinism
(`ellipse_fields_are_editable_in_place`,
`spline_fit_points_and_closure_are_editable`).

## Linear dimension editing

`updateDimension` edits one existing linear dimension in place. Only
`EntityKind::Dimension` is accepted in this version: a radial, diameter, ordinate
or angular dimension, and every other entity kind, reports
`dimension/unsupported-entity`. The entity keeps its id and layer, the edit is one
history step and the revision bumps exactly once.

The supplied fields `from`, `to`, `offset` and `style` replace only that value and
at least one field is required, so the text override and every omitted field stay
as they were. `from` and `to` must be finite and the resulting measured span must
be non-zero. `offset` is the signed distance of the dimension line from the
measured segment and must be finite. `style` is a dimension style id: it is
trimmed and must name a declared dimension style, and a missing value keeps the
current style.

| Code | Cause |
| :--- | :--- |
| `session/argument-invalid` | No field is supplied, a measured point or the offset is not finite, the resulting span is zero, or `style` is empty. |
| `dimension/unsupported-entity` | The id names a radial, diameter, ordinate or angular dimension, or another entity kind. |
| `object/not-found` | The id does not name an entity, or `style` names no declared dimension style. |
| `drawing/layer-locked` | The entity's layer is locked. |
| `drawing/layer-frozen` | The entity's layer is frozen. |

A rejected edit leaves the drawing and the revision untouched. The model tests
cover partial field edits with untouched values kept
(`linear_dimension_fields_and_style_are_editable_in_place`), a style assignment,
the zero-span, non-finite and empty-style refusals, unknown ids and styles, the
linear-only boundary, locked and frozen layers, the single revision bump, the
save/reopen round trip and determinism.

## Drawing diff

`diff(left, right)` reads two `.ocad` projects without opening a handle and
compares them by entity id and layer id. Added entries exist only in the right
project, removed entries only in the left, and changed entries carry the
differing `fields` plus both versions. Drawing-level `metadata` covers the
title, the unit and the declared properties. Each list stays in document order,
so the report is deterministic.

The report carries `{ identical, left, right, summary, entities, layers,
metadata }`, where `summary` counts entity, layer and metadata differences and
`left`/`right` describe each file. `identical` is true when every count is zero.
Revisions, dimension and text styles, line types, groups, layouts, draw order
and `nextEntity` are described per side but are not compared.

| Code | Cause |
| :--- | :--- |
| `drawing/not-found` | A file does not exist. |
| `drawing/io-error` | A file cannot be read. |
| `format/invalid` | A file is not a version-1 OpenCAD project. |
| `format/unsupported-version` | A file declares another version. |

The model tests cover identical projects (`diff_reports_identical_projects`),
added, removed, changed and metadata entries
(`diff_lists_added_removed_changed_and_metadata_entries`) and the unreadable and
malformed refusals (`diff_reports_unreadable_and_malformed_projects`).

## Boundaries that stay unsupported

- No DWG, PNG or JPG reading or writing. IGES is export-only; there is no
  reader.
- No full DXF reader or writer; only the subset above.
- No full SVG reader; only the declared element subset above.
- No interactive viewport, zoom/pan canvas or hover picking. The studio is a
  shell; SVG is the preview.
- No constraint solver, parameterisation or associativity between entities or
  dimensions.
- No 3D primitives, solids or booleans; STEP/STL/OBJ are faceted projections.
- No paper-space layout export to DXF, SVG or PDF.
- Snap quadrants, nearest points and perpendicular points are analytic for
  lines, polylines, circles and arcs only, and tangent points for circles and
  arcs; ellipses and splines stay endpoint, midpoint and centre snaps.
  Intersections cover lines, full circles and arcs, not ellipses or splines.
- `text find`/`replace` covers `TEXT` entities only, not text inside dimensions
  or hatches.
- Named UCS entities are not modelled; the UCS is a coordinate query, not a
  stored object.
- Dimension styles and text styles are limited to the fields exposed by the
  style commands.
- `dimension update` edits linear dimensions only. Radial, diameter, ordinate and
  angular dimensions report `dimension/unsupported-entity`, and the command never
  changes an entity's kind or creates a dimension.
- A linear dimension records an optional dimension style id that
  `dimension update --style` validates against the declared styles. SVG, PDF and
  DXF export still use their fixed label metrics, so the stored style does not
  change the drawn geometry yet.
- Fillet and the two-line `chamfer` accept two straight lines only. A bulk
  bevel of one polyline is `polylineChamfer` instead.
- Polyline chamfer bevels straight-segment polylines only. Every adjacent
  segment must be longer than twice the distance, there is no per-corner
  distance list, the first and last vertex of an open chain stay put, and it
  does not chamfer arcs, ellipses, splines or two-line corners.
- Trim cuts line, arc and circle targets with a line, circle or arc cutter, and
  a circle target becomes an arc. It does not trim polylines, splines, ellipses,
  block references or other entity types.
- Extend lengthens a line or an arc only, and its boundary must be a line,
  circle or arc. An arc target grows along its own circle and never wraps past
  a full turn. It does not extend polylines, circles, ellipses or splines, and
  it does not follow a boundary past its drawn extent.
- Break splits a line or an arc only; it does not break polylines, circles,
  ellipses, splines, text, hatches or dimensions, and it never turns a circle
  into an arc.
- Join merges two lines only, and only when they are collinear, share one
  endpoint and sit on the same layer. It does not join polylines, arcs, circles,
  ellipses, splines or chains of more than two lines, does not close a gap
  between separate endpoints, and does not move geometry.
- Align and distribute translate whole entities by their sampled bounding
  boxes; they never rotate, scale or resize an entity and do not refuse
  results whose boxes overlap.
- Offset covers lines, straight-segment polylines, circles and arcs only. It
  does not clip self-intersections, miter arc or bulge segments (polylines have
  none) or offset ellipses, splines, points, text, hatches and dimensions.
- Hatch patterns expand into straight line entities clipped to one polygonal
  boundary. There are no named pattern definitions, no island detection, no
  curved boundary clipping and no associative regeneration when the boundary
  later moves.
- Title blocks are plain line and text entities. They carry no attributes,
  fields, automatic date updates or reusable block definition, and they are not
  linked to the drawing properties after creation.
- Quantity tables are plain line and text entities. They are not editable table
  objects: cells carry no formula or override, the values are a snapshot taken
  at insert time, and a later geometry edit does not refresh the table. A long
  group label is not wrapped or clipped.
- `stats` groups by layer, entity kind, layer colour and layer line type only.
  It does not group by block, text content or arbitrary metadata, and curved
  lengths and areas follow the sampled measurement rules.
- Polyline smoothing is Chaikin corner cutting on a straight-segment polyline.
  It does not convert a polyline into a spline, fit a curve through the original
  vertices, simplify a chain, close an open chain or smooth arcs, ellipses,
  splines or hatches.
- Drawing diff matches entities and layers by their stable id only. It does not
  compare geometry by tolerance, styles, line types, groups, layouts, draw order
  or revisions, and it does not write a merged drawing.
- Valid colours are `#rrggbb`; lineweights are `0`–`10` mm; the line-type
  pattern holds at most 12 finite positive entries.
- The model does not enforce arbitrary user metadata; only the declared
  title-block properties are editable.
- `layer delete` removes an empty layer only and refuses the last layer or a
  layer still in use.
