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

## Arcs, ellipses and polylines

| Command | Options |
| :--- | :--- |
| `arc add` | `--center x,y`, `--radius n`, `--start deg`, `--end deg` (defaults to start + 360), `--layer` |
| `arc update` | `--id <id>`, at least one of `--radius`, `--start`, `--end` |
| `ellipse add` | `--center x,y`, `--rx n`, `--ry n`, `--rotation deg`, `--start deg`, `--end deg`, `--layer` |
| `polyline update` | `--id <id>`, then `--points "x,y ..."` or `--closed on\|off` |
| `polyline insert-vertex` | `--id <id>`, `--index n`, `--at x,y` |
| `polyline remove-vertex` | `--id <id>`, `--index n` |
| `polyline move-vertex` | `--id <id>`, `--index n`, `--at x,y` |

```bash
open-cad polyline insert-vertex plan.ocad --id entity-7 --index 2 --at 240,120
open-cad polyline move-vertex plan.ocad --id entity-7 --index 0 --at 0,0
open-cad polyline remove-vertex plan.ocad --id entity-7 --index 3
```

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
| `trim` | `--id <line>`, `--cutter <entity>` (a line, circle or arc) |
| `extend` | `--id <line>`, `--to <entity>` (a line, circle or arc), optional `--side end\|start` (default `end`) |
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

`trim` cuts the `--id` line at the `--cutter` line, circle or arc and saves the
file. One crossing keeps the longer side; two crossings keep the longer of the
two outer segments. A cutter without a valid crossing reports
`trim/no-intersection`; tangency, collinearity, a zero-length input or a cut on
an endpoint reports `trim/degenerate`.

`extend` lengthens the `--id` line until it meets the `--to` line, circle or
arc and saves the file. By default the `to` endpoint moves; `--side start` moves
the `from` endpoint instead. Only crossings ahead of the moving endpoint count
and the nearest one becomes the new endpoint. A boundary the ray never reaches
reports `extend/no-intersection`.

## Measure and select

| Command | Options |
| :--- | :--- |
| `measure` | `--ids a,b` (per entity) or `--sheet <name>` (drawing-wide within one layout) |
| `measure-points` | `--points "x,y x,y"` or `--points "x,y x,y x,y"` (two or three points) |
| `stats` | — |
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
value exits `2`. A failed inspection exits `1`.
