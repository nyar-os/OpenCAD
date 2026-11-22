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
| SVG | supported | Preview renderer and export target. |
| PDF | supported | Export target only; not a renderer and not editable. |
| STEP | partial | Export direction only (AP203 solids and faces). |
| OBJ | partial | Export direction only (triangle meshes). |
| STL | export target only | Recorded as `unsupported` under `formats` because there is no STL reader; `exportTargets.stl` is `supported`. |
| DWG, IGES/IGS, PNG, JPG | unsupported | No reader and no writer. |
| Interactive viewport | unsupported | SVG output is the only renderer; the studio does not paint geometry. |
| Constraint solver, 3D kernel | unsupported | No constraints, booleans, analytic surfaces or solids beyond faceted projection. |

Entity kinds reported by `entities`: `line`, `circle`, `arc`, `ellipse`,
`point`, `polyline`, `spline`, `text`, `dimension`, `radial-dimension`,
`ordinate-dimension`, `angular-dimension`, `hatch`.

Operations reported as supported: `transform`, `copy`, `array`, `explode`,
`offset`, `fillet`, `chamfer`, `measure`, `measure-entity`, `measure-points`,
`stats`, `dimension-style`, `text-style`, `text-search`, `group`, `select`,
`properties`, `snap`, `linetype`, `ucs`. `constraints` and `kernel3d` are
reported as unsupported.

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
- Unitless files assume millimetres and report it.

Losses are returned in `losses` as `{ code, severity, detail }`. `notice` means
geometry was kept with a documented approximation. `warning` means content was
skipped or replaced and the imported drawing differs from the source. Nothing is
dropped silently.

| Code | Severity | Meaning |
| :--- | :--- | :--- |
| `dxf/assumed-units` | notice | No `$INSUNITS`; millimetres assumed. |
| `dxf/layer-color` | warning | A true-colour layer colour was replaced. |
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

## DXF export

`dxf` writes R12-style ASCII and declares `AC1015` when the drawing has custom
line types, lineweights, a hatch, a spline, or a non-plottable layer; otherwise
it declares `AC1009`. It writes `$INSUNITS`, the LTYPE, LAYER, STYLE and DIMSTYLE
tables, LINE, CIRCLE, ARC, ELLIPSE, POINT, TEXT, LWPOLYLINE, SPLINE,
linear/radial/ordinate/angular DIMENSION, and a HATCH that uses its boundary
polygon with the pattern name only. `dxf-r12` always declares `AC1009` and always
reports omissions.

Reported losses: `dxf/r12-units`, `dxf/r12-lineweight`, `dxf/r12-plottable`,
`dxf/layer-color`, `dxf/r12-ellipse`, `dxf/r12-spline`,
`dxf/dimension-lines`, `dxf/hatch-boundary`, `dxf/r12-hatch`,
`dxf/hatch-pattern`. Hatch pattern geometry is never written, only its name.

Not written: `DWG`, blocks, `XREF`, images, layouts and paper space, proprietary
or proxy objects, text styles beyond the declared table fields, and any
3D content.

## SVG and PDF

Both formats skip hidden, frozen and non-plottable layers. They stroke every
visible layer at its lineweight — zero lineweight uses the SVG/PDF default — and
reduce the line type to one dash/gap pair so a dashed layer reads as dashed. They
draw hatch boundary polygons, not hatch patterns.

SVG is the only renderer. PDF is A4 landscape with one uniform fit to the page.
PDF approximates arcs and ellipses with cubic Bezier segments and reports
`pdf/text-encoding` when a character cannot be encoded. Neither format is
editable: reopening it is not supported.

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

## Boundaries that stay unsupported

- No DWG, IGES, PNG or JPG reading or writing.
- No full DXF reader or writer; only the subset above.
- No interactive viewport, zoom/pan canvas or hover picking. The studio is a
  shell; SVG is the preview.
- No constraint solver, parameterisation or associativity between entities or
  dimensions.
- No 3D primitives, solids or booleans; STEP/STL/OBJ are faceted projections.
- No paper-space layout export to DXF, SVG or PDF.
- No snap for arcs, circles or intersections; snapping is endpoint-based
  against lines and line-to-line intersections.
- `text find`/`replace` covers `TEXT` entities only, not text inside dimensions
  or hatches.
- Named UCS entities are not modelled; the UCS is a coordinate query, not a
  stored object.
- Dimension styles and text styles are limited to the fields exposed by the
  style commands.
- Fillet and chamfer accept two straight lines only.
- Valid colours are `#rrggbb`; lineweights are `0`–`10` mm; the line-type
  pattern holds at most 12 finite positive entries.
- The model does not enforce arbitrary user metadata; only the declared
  title-block properties are editable.
- `layer delete` removes an empty layer only and refuses the last layer or a
  layer still in use.
