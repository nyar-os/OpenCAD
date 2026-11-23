# `open-cad` command reference

```text
open-cad <command> [file] [options]
```

Commands that operate on a drawing take its path as the first positional
argument. `create` and `import` take no input file. Response text goes to
standard output; a domain failure prints its stable error code and exits `1`
(a failing inspection or a failed studio launch) or `2` (a usage problem).

## Session

| Command | Options | Behaviour |
| :--- | :--- | :--- |
| `capabilities` | — | Prints the capability matrix, checks and export targets. |
| `create` | `--out <file>` (required), `--title <text>`, `--units mm\|inch`, `--template <name>`, `--overwrite` | Saves a new drawing. `--template` is accepted but no template is loaded. Refuses an existing file unless `--overwrite`. |
| `open <file>` | — | Prints id, revision, entity count, unit, title and layer names. |
| `inspect <file>` | `--checks <a,b,...>` | Runs the selected checks; defaults to `units,layers,geometry,polylines,dimensions,duplicate-entity,zero-length,open-contour`. Exits `1` when any error is found. |
| `diff <left> <right>` | — | Compares two project files by entity and layer id and prints the counts and side-by-side differences. Exits `1` when the drawings differ. |
| `export <file>` | `--format dxf\|dxf-r12\|obj\|step\|stl\|svg\|pdf` (required), `--out <file>` (required) | Writes the format and prints its losses. |
| `undo <file>` | — | Restores the previous snapshot; persists through the sidecar history. |
| `redo <file>` | — | Reapplies the next snapshot. |
| `import <source.dxf>` | `--out <file>` (required), `--title <text>`, `--overwrite` | Imports the declared ASCII DXF subset and prints entity counts and losses. |
| `gui` | — | Opens the studio window and waits for it to close. |

## Entities

Every `--layer` defaults to `0`; dimension commands default to `dimensions`.
Coordinates are `x,y` in the drawing unit. Ids are comma-separated.

| Command | Options |
| :--- | :--- |
| `entity add-line` | `--from x,y`, `--to x,y`, `--layer` |
| `entity add-point` | `--at x,y`, `--layer` |
| `entity add-circle` | `--center x,y`, `--radius n`, `--layer` |
| `entity add-polyline` | `--points "x,y x,y ..."`, `--closed`, `--layer` |
| `entity add-spline` | `--points "x,y x,y ..."` (at least two fit points), `--closed`, `--layer` |
| `entity add-hatch` | `--points "x,y ..."`, `--pattern <name>`, `--layer` |
| `entity hatch-pattern` | `--id <entity>`, `--angle deg`, `--spacing n`, `--cross on\|off` |
| `entity add-rectangle` | `--from x,y`, `--to x,y`, `--layer` |
| `entity add-polygon` | `--center x,y`, `--radius n`, `--sides n`, `--rotation deg`, `--layer` |
| `entity add-text` | `--at x,y`, `--height n`, `--text <text>`, `--layer` |
| `entity add-dimension` | `--from x,y`, `--to x,y`, `--offset n`, `--text <text>`, `--layer` |
| `entity add-radial-dimension` | `--center x,y`, `--radius n`, `--angle deg`, `--offset n` (default 0), `--diameter`, `--text <text>`, `--layer` |
| `entity add-ordinate-dimension` | `--origin x,y`, `--feature x,y`, `--axis x\|y`, `--offset n` (default 0), `--text <text>`, `--layer` |
| `entity add-angular-dimension` | `--center x,y`, `--first x,y`, `--second x,y`, `--offset n`, `--text <text>`, `--layer` |
| `entity delete` | `--id <id>` |
| `entity list` | — |
| `entity move` | `--id <id>`, then `--to x,y` or `--dx n` and/or `--dy n` (not both forms) |
| `entity update` | `--id <id>`, at least one of `--text`, `--height`, `--radius` |
| `entity set-layer` | `--id <id>`, `--to <layer>` |

`entity update` changes a `TEXT` height or text, or a circle radius; other kinds
answer `object/unsupported`. `entity move` applies to any entity with a centre
or start point.

## Arcs, ellipses, polylines and splines

| Command | Options |
| :--- | :--- |
| `arc add` | `--center x,y`, `--radius n`, `--start deg`, `--end deg` (defaults to start + 360), `--layer` |
| `arc update` | `--id <id>`, at least one of `--radius`, `--start`, `--end` |
| `ellipse add` | `--center x,y`, `--rx n`, `--ry n`, `--rotation deg`, `--start deg`, `--end deg`, `--layer` |
| `ellipse update` | `--id <id>`, at least one of `--rx`, `--ry`, `--rotation`, `--start`, `--end` |
| `polyline update` | `--id <id>`, then `--points "x,y ..."` or `--closed on\|off` |
| `polyline insert-vertex` | `--id <id>`, `--index n`, `--at x,y` |
| `polyline remove-vertex` | `--id <id>`, `--index n` |
| `polyline move-vertex` | `--id <id>`, `--index n`, `--at x,y` |
| `polyline smooth` | `--id <id>`, `--iterations n` (default 2, 1-8), `--ratio n` (default 0.25, between 0 and 0.5) |
| `polyline chamfer` | `--id <id>`, `--distance n` (each adjacent segment must be longer than `2 * n`) |
| `spline update` | `--id <id>`, then `--points "x,y ..."` (at least two fit points) or `--closed on\|off` |
| `spline insert-vertex` | `--id <id>`, `--index n`, `--at x,y` |
| `spline remove-vertex` | `--id <id>`, `--index n` |
| `spline move-vertex` | `--id <id>`, `--index n`, `--at x,y` |

```bash
open-cad polyline insert-vertex plan.ocad --id entity-7 --index 2 --at 240,120
open-cad polyline move-vertex plan.ocad --id entity-7 --index 0 --at 0,0
open-cad polyline remove-vertex plan.ocad --id entity-7 --index 3
open-cad polyline smooth plan.ocad --id entity-7 --iterations 3
open-cad polyline chamfer plan.ocad --id entity-7 --distance 4
open-cad ellipse update plan.ocad --id entity-4 --rx 60 --rotation 15
open-cad spline insert-vertex plan.ocad --id entity-9 --index 1 --at 40,25
```

`ellipse update` changes only the supplied fields of one ellipse and saves the
file. At least one of `--rx`, `--ry`, `--rotation`, `--start` and `--end` is
required; omitted fields keep their value and the entity keeps its id and layer.
A non-positive `--rx`/`--ry` or a non-finite value reports
`session/argument-invalid`, and a non-ellipse reports `object/unsupported`.

`spline update`, `spline insert-vertex`, `spline remove-vertex` and `spline
move-vertex` edit the fit points of one spline exactly like their polyline
counterparts and save the file. `--points` replaces the list, `--closed` sets the
flag, `--index` is zero-based and insertion accepts the current fit-point count
to append. A spline always keeps at least two fit points: a replacement list with
fewer, or a removal that would leave fewer, reports `session/argument-invalid`.
A non-spline reports `object/unsupported`; an unknown id reports
`object/not-found`; a locked or frozen layer reports `drawing/layer-locked` or
`drawing/layer-frozen`.

## Dimensions

| Command | Options |
| :--- | :--- |
| `dimension update` | `--id <id>`, at least one of `--from x,y`, `--to x,y`, `--offset n` or `--style <id>` |

```bash
open-cad dimension update plan.ocad --id entity-12 --to 12000,0
open-cad dimension update plan.ocad --id entity-12 --offset 600 --style arch
```

`dimension update` changes only the supplied fields of one linear dimension and
saves the file. `--from`/`--to` move the measured points, `--offset` sets the
signed distance of the dimension line from the measured segment, and `--style`
assigns a declared dimension style id. Omitted fields keep their value, so the
text override, id and layer are preserved.

A non-finite point or offset, a resulting zero-length span, or an empty
`--style` reports `session/argument-invalid`; an unknown `--style` or `--id`
reports `object/not-found`; a locked or frozen layer reports
`drawing/layer-locked` or `drawing/layer-frozen`. Only linear dimensions are
editable in this version: a radial, diameter, ordinate or angular dimension, or
another entity kind, reports `dimension/unsupported-entity`.

## Edit, copy and transform

`--ids` takes a comma-separated list. `transform` needs exactly one selector and
exactly one transform.

| Command | Options |
| :--- | :--- |
| `transform` | selector: `--ids a,b` or `--layer <name>` or `--all`; transform: `--translate x,y` or `--rotate deg --center x,y` or `--scale factor --center x,y` or `--mirror x\|y --mirror-at n` |
| `copy` | `--ids a,b`, `--dx n`, `--dy n`, `--count n` |
| `array` | `--ids a,b`, then `--rows n --columns n [--row-spacing n --column-spacing n]` or `--center x,y --count n --angle deg` |
| `explode` | `--id <id>` (a block reference; unsupported for other kinds) |
| `offset` | `--id <entity>`, `--distance n`, optional `--side left\|right\|outside\|inside` |
| `trim` | `--id <line\|arc\|circle>`, `--cutter <entity>` (a line, circle or arc) |
| `extend` | `--id <line\|arc>`, `--to <entity>` (a line, circle or arc), optional `--side end\|start` (default `end`) |
| `break` | `--id <line\|arc>`, `--at x,y` (strictly inside the segment or arc sweep) |
| `join` | `--a <line>`, `--b <line>` (collinear and sharing one endpoint) |
| `align` | `--ids a,b`, `--mode left\|center\|right\|bottom\|middle\|top` |
| `distribute` | `--ids a,b,c`, `--axis x\|y` |
| `fillet` | `--first <id>`, `--second <id>`, `--radius n` |
| `chamfer` | `--first <id>`, `--second <id>`, `--distance-a n`, `--distance-b n` (defaults to A) |

```bash
open-cad transform plan.ocad --layer Structure --translate 100,0
open-cad copy plan.ocad --ids entity-3,entity-4 --dx 500 --dy 0 --count 3
open-cad array plan.ocad --ids entity-3 --rows 2 --columns 3 --row-spacing 400
open-cad offset plan.ocad --id entity-3 --distance 5
open-cad offset plan.ocad --id entity-3 --distance 5 --side left
open-cad trim plan.ocad --id entity-3 --cutter entity-4
open-cad extend plan.ocad --id entity-3 --to entity-4
open-cad extend plan.ocad --id entity-3 --to entity-4 --side start
open-cad break plan.ocad --id entity-3 --at 50,0
open-cad join plan.ocad --a entity-3 --b entity-4
open-cad align plan.ocad --ids entity-3,entity-4,entity-5 --mode left
open-cad distribute plan.ocad --ids entity-3,entity-4,entity-5 --axis x
open-cad fillet plan.ocad --first entity-5 --second entity-6 --radius 20
```

`offset` appends one new entity on the source layer and leaves the source in
place. Without `--side` it keeps the signed convention: a positive
`--distance` grows a circle or arc and offsets a line or polyline to the left,
while a negative distance shrinks or offsets right. With `--side`, the distance
must be positive and finite; `--side left` and `--side right` apply to a line or
polyline and measure along its travel direction, while `--side outside` and
`--side inside` apply to a circle or arc and change only the radius. A polyline
corner that would need a miter longer than four times the distance is refused
when `--side` is used.

`trim` cuts the `--id` line, arc or circle at the `--cutter` line, circle or arc
and saves the file. A line or arc target keeps the longer side for one crossing
and the longer of the two outer segments for two. A circle target needs exactly
two crossings and becomes an arc over the longer sweep. A cutter without a valid
crossing reports `trim/no-intersection`; tangency, collinearity, a zero-length
or zero-radius input, a single circle crossing or a cut on an endpoint reports
`trim/degenerate`.

`extend` lengthens the `--id` line or arc until it meets the `--to` line, circle
or arc and saves the file. By default the `to` endpoint moves and `--side start`
moves the `from` endpoint instead. An arc target travels along its own circle:
`--side end` walks counter-clockwise from the end angle and `--side start` walks
clockwise from the start angle. Only crossings ahead of the moving endpoint
count and the nearest one becomes the new endpoint, and an arc never wraps past
a full turn. A boundary the extension never reaches reports
`extend/no-intersection`.

`break` splits the `--id` line or arc into two pieces at `--at x,y` and saves the
file. For a line the point must lie strictly inside the segment, away from both
endpoints; a point close to the line is projected onto it so both pieces stay
collinear. The source is removed and replaced by two lines with fresh ids on the
same layer. For an arc the point must lie on the arc, away from both endpoints:
the source keeps the part from its start angle to the cut angle and a new arc
with the next id covers the rest, both sharing the centre and radius. A point on
an endpoint or off the entity reports `break/not-on-entity`; a circle or another
kind reports `object/unsupported`.

`join` merges the `--a` and `--b` lines and saves the file. Both must be lines
on the same layer, one endpoint of each must coincide, and the lines must be
collinear and continue in opposite directions from that shared point; the
tolerance is `1e-9` scaled by the longer segment. The two sources are removed and
replaced by one line from `a`'s free endpoint to `b`'s free endpoint with a fresh
id. Endpoints that do not meet, lines that are not collinear, and a folded-back
overlap report `join/not-adjacent`; a non-line or a cross-layer pair reports
`object/unsupported`.

`align` moves the `--ids` entities onto one bounding-box edge or centreline
chosen by `--mode` and saves the file. `left`, `right`, `bottom` and `top` pull
the extreme edges of every box onto the smallest or largest edge of the
selection; `center` and `middle` pull the horizontal or vertical box centres
onto the midpoint of the selection's span. Entities move by translation only;
fewer than two distinct ids report `align/insufficient-entities`.

`distribute` spreads the `--ids` entities along `--axis x|y` with equal gaps
between consecutive bounding boxes and saves the file. Entities are ordered by
box centre; the first and last stay put and every interior entity is translated
so the gaps match. Fewer than three distinct ids report
`distribute/insufficient-entities`. Both commands refuse unknown ids with
`object/not-found`, locked or frozen layers with `drawing/layer-locked` or
`drawing/layer-frozen`, and entities without finite bounds with
`object/degenerate`.

## Hatch patterns and title blocks

`entity hatch-pattern` fills one hatch with real line entities clipped to its
boundary and saves the file. `--angle` (default `0`) sets the direction,
`--spacing` (default `5`) the distance between lines and `--cross on` adds a
second family perpendicular to the first. The lines land on the hatch layer, so
every exporter carries the fill. A solid fill reports
`hatch/solid-pattern`; a non-hatch id reports `hatch/unsupported-entity`; a
zero-area boundary reports `hatch/degenerate-boundary`; a non-positive
`--spacing` reports `hatch/invalid-spacing`.

`title-block` adds a sheet frame and a title block to a drawing and saves the
file. The sheet defaults to A3 landscape, the frame to a 10-unit margin and the
block to the `titleblock` layer, which is created when missing. The block
carries Title, Number, Revision, Scale, Date, Author and Sheet; each can be
overridden and an empty override leaves the cell blank. A sheet too small for
the frame reports `titleblock/invalid-sheet`.

```bash
open-cad entity hatch-pattern plan.ocad --id entity-7 --angle 45 --spacing 4
open-cad entity hatch-pattern plan.ocad --id entity-7 --spacing 6 --cross on
open-cad title-block plan.ocad --title "Pump skid" --number A-101 --revision C --scale 1:50 --date 2026-11-23
open-cad title-block plan.ocad --layer Sheet --width 200 --height 150 --margin 8
```

`polyline smooth` rounds the corners of one polyline with Chaikin corner
cutting and saves the file in place. Each pass replaces every segment with the
two points at `--ratio` and `1 - ratio` along it, so an open chain keeps its
endpoints and a closed chain keeps its closed flag. The entity keeps its id and
layer. `--iterations` defaults to `2` and accepts `1`-`8`; `--ratio` defaults to
`0.25` and must lie strictly between `0` and `0.5`. A non-polyline reports
`polyline/unsupported-entity`; a chain with fewer than three vertices reports
`polyline/degenerate`; an out-of-range pass count or ratio reports
`polyline/invalid-iterations` or `polyline/invalid-ratio`.

`polyline chamfer` bevels every corner of one polyline and saves the file in
place. Every interior vertex of an open chain, and every vertex of a closed
chain, is trimmed `--distance` along both adjacent segments: the corner is
replaced by the two trim points, so the entity stays one polyline and keeps its
id, layer and closed flag. `--distance` must be positive and finite, and every
adjacent segment must be longer than `2 * --distance`; a shorter or exactly
equal segment makes the whole edit fail with `session/argument-invalid` and
nothing is written. A non-polyline reports `polyline/unsupported-entity` and a
chain with fewer than three vertices reports `polyline/degenerate`. The
two-line `chamfer` command is separate.

## Comparing projects

`diff <left> <right>` reads both project files without opening a handle and
matches entities and layers by their stable id. It prints each side's path,
revision and counts, then either `identical` or the number of added, removed and
changed entities, changed layers and metadata fields. Entity changes list the
differing `fields` (`layer`, `geometry`), layer changes list the differing
style fields and metadata covers the title, unit and declared properties.
Revisions, styles, line types, groups, layouts and draw order are not compared.
The command exits `0` for identical drawings and `1` when anything differs;
a missing or malformed file reports the same codes as `open`.

```bash
open-cad diff revision-a.ocad revision-b.ocad
open-cad --json diff current.ocad baseline.ocad > diff.json
```

## Measure and select

| Command | Options |
| :--- | :--- |
| `measure` | `--ids a,b` (per entity) or `--sheet <name>` (drawing-wide within one layout) |
| `measure-points` | `--points "x,y x,y"` or `--points "x,y x,y x,y"` (two or three points) |
| `stats` | `--group-by layer\|type\|color\|linetype` (defaults to `layer`) |
| `quantity-table` | `--group-by layer\|type\|color\|linetype`, `--layer <name>`, `--origin x,y`, `--text-height n`, `--title <text>` |
| `select` | `--box x1,y1,x2,y2`, `--crossing`, `--layer <name>` |
| `snap` | `--modes <list>`, `--near x,y`, `--tolerance n`, `--layer <name>`, `--ids a,b` |
| `ucs` | `--origin x,y`, `--rotation deg`, `--direction to-world\|to-ucs`, `--points "x,y ..."`, `--ids a,b` (at least one of points or ids) |

`measure` with ids reports per-entity length, area, bounds or angle; with a
sheet, it reports drawing-wide totals for that layout. `measure-points` reports
distance, angle and area for the supplied points. `stats` counts entities,
layers and each kind. `select` reports ids inside or crossing the box,
optionally filtered by layer. `snap` reports snap candidates for the requested
modes — `endpoint`, `midpoint`, `center`, `quadrant`, `nearest`, `intersection`,
`perpendicular` and `tangent`, defaulting to
`endpoint,midpoint,center,intersection` — where `--near` supplies the probe
point, `--tolerance` keeps only candidates within a distance of it, and
`--layer`/`--ids` limit the source entities. `nearest`, `perpendicular`,
`tangent` and `--tolerance` all need `--near`. `tangent` reports the contact
points of the tangents from the probe to a circle or arc. `ucs` converts points
or entity positions between world and the configured UCS.

```bash
open-cad snap plan.ocad --modes quadrant,nearest --near 120,80 --tolerance 15
open-cad snap plan.ocad --modes perpendicular --near 120,80 --ids entity-3
open-cad snap plan.ocad --modes tangent --near 120,80 --ids entity-3
open-cad snap plan.ocad --modes intersection --ids entity-3,entity-4
```

`stats` lists one aggregate per group with the entity count, path length and
enclosed area, plus a total. `--group-by layer` keeps the layer order and
includes empty layers; `type`, `color` and `linetype` list only keys that occur,
sorted by key. Length and area follow `measure`.

`quantity-table` draws those aggregates as a framed table and saves the file. A
row per group is followed by a total row; a caption row sits above the header
unless `--title ""` suppresses it. The bottom-left corner comes from `--origin`
(default `0,0`), the width is `60 × --text-height` (default `3.5`) and every row
is `2.5 × --text-height` tall. Everything is a real line or text entity on
`--layer` (default `quantity`, created when missing), so SVG, PDF and DXF carry
it. Rows are computed before the table is inserted, so a layer grouping never
lists the table itself. A non-positive `--text-height`, a non-finite `--origin`
or a caption longer than the property limit reports `session/argument-invalid`;
a locked or frozen layer reports `drawing/layer-locked` or
`drawing/layer-frozen`.

```bash
open-cad stats plan.ocad --group-by type
open-cad quantity-table plan.ocad --group-by layer --origin 0,-2500 --text-height 120
open-cad quantity-table plan.ocad --group-by type --layer Schedule --title "Bill of quantities"
```

## Layers

| Command | Options |
| :--- | :--- |
| `layer add` | `--name <name>`, `--color "#rrggbb"` |
| `layer rename` | `--name <name>`, `--to <name>` |
| `layer visible` | `--name <name>`, `--visible on\|off` |
| `layer lock` | `--name <name>`, `--locked on\|off` |
| `layer freeze` | `--name <name>`, `--frozen on\|off` |
| `layer plot` | `--name <name>`, `--plottable on\|off` |
| `layer set` | `--name <name>`, at least one of `--linetype <name>`, `--lineweight n` (0–10 mm) |
| `layer merge` | `--from <name>`, `--to <name>` |
| `layer delete` | `--name <name>` (empty layer only; the last layer is refused) |

## Styles, text, groups and properties

| Command | Options |
| :--- | :--- |
| `dimstyle list` | — |
| `dimstyle add` | `--name`, `--scale`, `--arrow-size`, `--text-height`, `--extension`, `--origin-offset`, `--precision 0..8` |
| `dimstyle update` | `--name`, `--to`, and any field from `add` |
| `dimstyle delete` | `--name` |
| `textstyle list` | — |
| `textstyle add` | `--name`, `--font`, `--height`, `--width-factor`, `--oblique` |
| `textstyle update` | `--name`, `--to`, and any field from `add` |
| `textstyle delete` | `--name` |
| `linetype list` | — |
| `linetype add` | `--name`, `--description`, `--pattern "5,2"` |
| `linetype update` | `--name`, `--to`, `--description`, `--pattern` |
| `linetype delete` | `--name` |
| `group list` | — |
| `group create` | `--name`, `--ids a,b` |
| `group add` | `--name`, `--ids a,b` |
| `group remove` | `--name`, `--ids a,b` |
| `group delete` | `--name` |
| `text find` | `--pattern <text>`, `--match-case`, `--layer <name>` |
| `text replace` | `--pattern <text>`, `--to <text>`, `--match-case`, `--layer <name>` |
| `properties show` | — |
| `properties set` | any of `--title`, `--author`, `--description`, `--company`, `--project`, `--keywords` |
| `units convert` | `--to mm\|inch` |

```bash
open-cad linetype add plan.ocad --name DASHED --description "Dashed" --pattern "5,2"
open-cad layer set plan.ocad --name Structure --linetype DASHED --lineweight 0.5
open-cad text replace plan.ocad --pattern "TBD" --to "Studio"
```

## Errors

Domain errors print a stable code (for example `object/not-found`,
`drawing/layer-locked`, `capability/not-implemented`). A missing file, flag or
value exits `2`. A failed inspection exits `1`, and so does `diff` when the two
drawings differ.
