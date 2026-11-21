/**
 * Declared types of `@wae/open-cad`.
 *
 * The shapes below describe the real MVP document: a drawing holds ordered
 * layers and a flat entity list, and every report carries the revision it was
 * produced from. Editing requests are explicit so a caller never relies on a
 * positional convention.
 */

import type { DrawingFormat } from './formats.js';

/** Opaque drawing identity. */
export type DrawingId = string;
/** Opaque layer identity. */
export type LayerId = string;
/** Opaque entity identity. */
export type EntityId = string;

/** A point `[x, y]` in drawing units. */
export type Point = readonly [number, number];

/** Disclosure level. `partial` always ships a loss path and diagnostic codes. */
export type SupportLevel = 'supported' | 'partial' | 'unsupported';

/** Inspection rules `OpenCAD.inspect()` may run. */
export type InspectCheck =
    | 'units'
    | 'layers'
    | 'empty-layer'
    | 'geometry'
    | 'polylines'
    | 'constraints'
    | 'dimensions'
    | 'references'
    | 'duplicate-entity'
    | 'zero-length'
    | 'open-contour';

/** Entity kinds in the capability matrix. */
export type EntityKindName =
    | 'line'
    | 'circle'
    | 'arc'
    | 'ellipse'
    | 'point'
    | 'polyline'
    | 'text'
    | 'dimension'
    | 'radial-dimension'
    | 'ordinate-dimension'
    | 'hatch';

/** Edit operations in the capability matrix. */
export type OperationName =
    | 'transform'
    | 'copy'
    | 'array'
    | 'explode'
    | 'offset'
    | 'measure'
    | 'measure-entity'
    | 'stats'
    | 'dimension-style'
    | 'text-style'
    | 'text-search'
    | 'group'
    | 'select'
    | 'properties'
    | 'snap'
    | 'constraints'
    | 'kernel3d';

/** Static render targets. */
export type RenderFormat = 'svg' | 'png' | 'pdf';

/** Exchange formats `export()` can write. */
export type ExportFormat = 'dxf' | 'obj' | 'step' | 'stl' | 'svg' | 'pdf';

/** Hard limits applied before work starts. */
export interface ResourceBudgets {
    readonly maxFileBytes?: number;
    readonly maxEntities?: number;
}

/** Structured diagnostic emitted during open, inspection or export. */
export interface DrawingDiagnostic {
    readonly code: string;
    readonly severity: 'info' | 'warning' | 'error';
    readonly revision: number;
    readonly layerId?: LayerId;
    readonly entityId?: EntityId;
    readonly message: string;
}

/**
 * Where undo/redo snapshots live.
 *
 * `memory` keeps them inside the native handle, so they only survive one
 * session. `sidecar` also records each replaced document beside the project
 * file (`<file>.history.json`) so undo and redo work across processes.
 */
export type HistoryMode = 'memory' | 'sidecar';

/** Session options. Constructing a session opens no window, file or socket. */
export interface OpenCADOptions {
    /** BCP-47 tag used for diagnostics and default copy. Defaults to `en`. */
    readonly locale?: string;
    /** Undo/redo backing store. Defaults to `memory`. */
    readonly history?: HistoryMode;
    readonly budgets?: ResourceBudgets;
    readonly diagnostics?: (event: DrawingDiagnostic) => void;
}

/** Result of `OpenCAD.capabilities()`. Read this instead of assuming a method works. */
export interface OpenCADCapabilities {
    readonly engine: { readonly package: string; readonly version: string };
    readonly project: SupportLevel;
    readonly formats: Readonly<Record<DrawingFormat, SupportLevel>>;
    readonly checks: Readonly<Record<InspectCheck, SupportLevel>>;
    readonly renderers: Readonly<Record<RenderFormat, SupportLevel>>;
    readonly exportTargets: Readonly<Record<ExportFormat, SupportLevel>>;
    /** Modelled entity kinds. */
    readonly entities: Readonly<Record<EntityKindName, SupportLevel>>;
    /** Wired edit operations; constraint solving and a 3D kernel stay unsupported. */
    readonly operations: Readonly<Record<OperationName, SupportLevel>>;
}

/** Options of `OpenCAD.create()`. */
export interface DrawingCreateOptions {
    readonly title?: string;
    /** Declared drawing unit, `mm` or `inch`. Defaults to `mm`. */
    readonly units?: 'mm' | 'inch';
    /** Accepted for API compatibility; the MVP does not load a template. */
    readonly template?: string;
}

/**
 * Input of `OpenCAD.open()`.
 *
 * `path` is the only accepted source so the engine never guesses whether a
 * string is a file name or serialized content.
 */
export interface DrawingInput {
    readonly path: string;
}

/** One layer of a drawing summary. */
export interface LayerSummary {
    readonly id: LayerId;
    readonly name: string;
    readonly color: string;
    readonly visible: boolean;
    readonly locked: boolean;
    readonly entityCount: number;
}

/**
 * Descriptive title-block properties of a drawing.
 *
 * An empty string means the field is not set. Changing these values never
 * moves geometry or changes a layer.
 */
export interface DrawingProperties {
    readonly author: string;
    readonly description: string;
    readonly company: string;
    readonly project: string;
    readonly keywords: string;
}

/** Result of `properties()`. */
export interface DrawingPropertiesResult {
    readonly revision: number;
    readonly title: string | null;
    readonly properties: DrawingProperties;
}

/** Arguments of `updateProperties()`. Omitted fields keep their value; an empty string clears one. */
export interface UpdatePropertiesRequest {
    /** Drawing title; an empty string removes it. */
    readonly title?: string;
    readonly author?: string;
    readonly description?: string;
    readonly company?: string;
    readonly project?: string;
    readonly keywords?: string;
}

/** Result of `updateProperties()`. */
export interface DrawingPropertiesChange {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly title: string | null;
    readonly properties: DrawingProperties;
}

/** One named dimension style; mirrors a DXF `DIMSTYLE` table entry. */
export interface DimensionStyle {
    readonly id: string;
    readonly name: string;
    /** Overall scale applied to the style's lengths. */
    readonly scale: number;
    /** Arrowhead size in drawing units. */
    readonly arrowSize: number;
    /** Dimension text height in drawing units. */
    readonly textHeight: number;
    /** Extension line length beyond the dimension line. */
    readonly extension: number;
    /** Gap between a measured point and its extension line. */
    readonly originOffset: number;
    /** Decimal places in a measured label, 0-8. */
    readonly precision: number;
}

/** One named text style; mirrors a DXF `STYLE` table entry. */
export interface TextStyle {
    readonly id: string;
    readonly name: string;
    /** Primary font file or family name. */
    readonly font: string;
    /** Fixed cap height; 0 leaves the height to each text entity. */
    readonly height: number;
    /** Horizontal width factor; 1 is normal. */
    readonly widthFactor: number;
    /** Oblique angle in degrees. */
    readonly oblique: number;
}

/** Summary of one drawing at its committed revision. */
export interface Drawing {
    readonly id: DrawingId;
    readonly revision: number;
    readonly title: string | null;
    /** Declared drawing unit, `mm` or `inch`. */
    readonly units: 'mm' | 'inch';
    readonly layers: readonly LayerSummary[];
    /** Named dimension styles; a drawing always keeps at least one. */
    readonly dimStyles: readonly DimensionStyle[];
    /** Named text styles; a drawing always keeps at least one. */
    readonly textStyles: readonly TextStyle[];
    /** Number of named entity groups. */
    readonly groups: number;
    /** Descriptive title-block properties. */
    readonly properties: DrawingProperties;
    readonly entities: number;
    /** Whether the handle still has an earlier snapshot to restore. */
    readonly canUndo: boolean;
    /** Whether an undone snapshot can be reapplied. */
    readonly canRedo: boolean;
}

/** Result of `listDimStyles()`. */
export interface DimensionStyleList {
    readonly revision: number;
    readonly dimStyles: readonly DimensionStyle[];
}

/** Arguments of `addDimStyle()`. Omitted numbers keep the standard defaults. */
export interface AddDimensionStyleRequest {
    readonly name: string;
    readonly scale?: number;
    readonly arrowSize?: number;
    readonly textHeight?: number;
    readonly extension?: number;
    readonly originOffset?: number;
    readonly precision?: number;
}

/** Arguments of `updateDimStyle()`. */
export interface UpdateDimensionStyleRequest {
    readonly styleId: string;
    readonly name?: string;
    readonly scale?: number;
    readonly arrowSize?: number;
    readonly textHeight?: number;
    readonly extension?: number;
    readonly originOffset?: number;
    readonly precision?: number;
}

/** Result of adding or updating one dimension style. */
export interface DimensionStyleChange {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly style: DimensionStyle;
}

/** Result of deleting one dimension style. */
export interface DimensionStyleDeletion {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly deleted: string;
}

/** Result of `listTextStyles()`. */
export interface TextStyleList {
    readonly revision: number;
    readonly textStyles: readonly TextStyle[];
}

/** Arguments of `addTextStyle()`. Omitted values keep the standard defaults. */
export interface AddTextStyleRequest {
    readonly name: string;
    /** Primary font file or family name; defaults to `txt`. */
    readonly font?: string;
    /** Fixed cap height; defaults to 0, which leaves the height to the entity. */
    readonly height?: number;
    /** Horizontal width factor; defaults to 1. */
    readonly widthFactor?: number;
    /** Oblique angle in degrees, -85 to 85; defaults to 0. */
    readonly oblique?: number;
}

/** Arguments of `updateTextStyle()`. */
export interface UpdateTextStyleRequest {
    readonly styleId: string;
    readonly name?: string;
    readonly font?: string;
    readonly height?: number;
    readonly widthFactor?: number;
    readonly oblique?: number;
}

/** Result of adding or updating one text style. */
export interface TextStyleChange {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly style: TextStyle;
}

/** Result of deleting one text style. */
export interface TextStyleDeletion {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly deleted: string;
}

/** Geometry of one entity. */
export type EntityKind =
    | { readonly type: 'line'; readonly from: Point; readonly to: Point }
    | { readonly type: 'circle'; readonly center: Point; readonly radius: number }
    | {
          readonly type: 'arc';
          readonly center: Point;
          readonly radius: number;
          /** Degrees, measured from the positive X axis. */
          readonly start_angle: number;
          /** Degrees; the arc sweeps counter-clockwise from the start angle. */
          readonly end_angle: number;
      }
    | {
          readonly type: 'ellipse';
          readonly center: Point;
          readonly rx: number;
          readonly ry: number;
          /** Major-axis rotation in degrees. */
          readonly rotation: number;
          /** Start parameter in degrees, measured from the major axis. */
          readonly start_angle: number;
          /** End parameter in degrees, swept counter-clockwise. */
          readonly end_angle: number;
      }
    | { readonly type: 'point'; readonly at: Point }
    | { readonly type: 'polyline'; readonly points: readonly Point[]; readonly closed: boolean }
    | { readonly type: 'text'; readonly at: Point; readonly height: number; readonly content: string }
    | { readonly type: 'dimension'; readonly from: Point; readonly to: Point; readonly offset: number; readonly text: string | null }
    | {
          readonly type: 'radial_dimension';
          readonly center: Point;
          readonly radius: number;
          /** Degrees of the measured point from the positive X axis. */
          readonly angle: number;
          /** Signed distance from the measured point to the label. */
          readonly offset: number;
          /** Whether the dimension measures the diameter instead of the radius. */
          readonly diameter: boolean;
          readonly text: string | null;
      }
    | {
          readonly type: 'ordinate_dimension';
          readonly origin: Point;
          readonly feature: Point;
          /** Axis the dimension measures. */
          readonly axis: 'x' | 'y';
          /** Signed perpendicular distance of the dimension line from the origin. */
          readonly offset: number;
          readonly text: string | null;
      }
    | {
          readonly type: 'hatch';
          /** Boundary vertices in order; the closing edge is implicit. */
          readonly points: readonly Point[];
          /** Pattern name kept as a description; `SOLID` is a solid fill. */
          readonly pattern: string;
      };

/** One drawable object. */
export interface Entity {
    readonly id: EntityId;
    readonly layer: LayerId;
    readonly kind: EntityKind;
}

/**
 * One named group of entities.
 *
 * Membership is metadata: grouping never changes geometry, layers or entity
 * revisions, and every member id points at an entity of the same drawing.
 */
export interface EntityGroup {
    readonly id: string;
    readonly name: string;
    readonly entities: readonly EntityId[];
}

/** Result of `listGroups()`. */
export interface GroupList {
    readonly revision: number;
    readonly groups: readonly EntityGroup[];
}

/** Arguments of `createGroup()`. */
export interface CreateGroupRequest {
    /** Group name; also becomes the group id. */
    readonly name: string;
    /** At least one existing entity id. */
    readonly entities: readonly EntityId[];
}

/** Arguments of `addGroupMembers()` and `removeGroupMembers()`. */
export interface GroupMembersRequest {
    readonly groupId: string;
    readonly entities: readonly EntityId[];
}

/** Result of creating or updating one group. */
export interface GroupChange {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly group: EntityGroup;
}

/** Result of deleting one group. */
export interface GroupDeletion {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly deleted: string;
}

/** How a rectangular selection treats entities that touch its boundary. */
export type SelectionMode = 'window' | 'crossing';

/** Arguments of `select()`. */
export interface SelectionRequest {
    /** Two opposite corners `[x, y]` of the selection rectangle. */
    readonly window: readonly [Point, Point];
    /** `window` keeps fully contained entities; `crossing` also keeps touching ones. Defaults to `window`. */
    readonly mode?: SelectionMode;
    /** Restrict the result to one layer id. */
    readonly layer?: LayerId;
}

/** Result of `select()`. */
export interface SelectionResult {
    readonly revision: number;
    readonly mode: SelectionMode;
    readonly window: readonly [Point, Point];
    readonly layer: string | null;
    readonly count: number;
    readonly ids: readonly EntityId[];
}

/** One computed object snap candidate. */
export type SnapMode = 'endpoint' | 'midpoint' | 'center' | 'intersection';

/** One snap candidate returned by `snap()`. */
export interface SnapPoint {
    readonly kind: SnapMode;
    readonly point: Point;
    /** Entities that produced the candidate; an intersection names both. */
    readonly entities: readonly EntityId[];
    /** Distance to the probe point; present only when `point` was given. */
    readonly distance?: number;
}

/** Arguments of `snap()`. */
export interface SnapRequest {
    /** Modes to compute; defaults to every mode. */
    readonly modes?: readonly SnapMode[];
    /** Probe point; candidates are sorted by distance to it. */
    readonly point?: Point;
    /** Keep only candidates within this distance; requires `point`. */
    readonly tolerance?: number;
    /** Restrict the query to one layer id. */
    readonly layer?: LayerId;
    /** Restrict the query to these entity ids. */
    readonly ids?: readonly EntityId[];
}

/** Result of `snap()`. */
export interface SnapResult {
    readonly revision: number;
    readonly modes: readonly SnapMode[];
    readonly point: Point | null;
    readonly tolerance: number | null;
    readonly count: number;
    /** True when more candidates existed than the limit of 1000. */
    readonly truncated: boolean;
    readonly points: readonly SnapPoint[];
}

/** Arguments of `addLine()`. */
export interface AddLineRequest {
    readonly layer: LayerId;
    readonly from: Point;
    readonly to: Point;
}

/** Arguments of `addCircle()`. */
export interface AddCircleRequest {
    readonly layer: LayerId;
    readonly center: Point;
    readonly radius: number;
}

/** Arguments of `addArc()`. */
export interface AddArcRequest {
    readonly layer: LayerId;
    readonly center: Point;
    readonly radius: number;
    /** Degrees, measured from the positive X axis. */
    readonly startAngle: number;
    /** Degrees; defaults to `startAngle + 360`. */
    readonly endAngle?: number;
}

/** Arguments of `addEllipse()`. A full ellipse uses `0` and `360`. */
export interface AddEllipseRequest {
    readonly layer: LayerId;
    readonly center: Point;
    readonly rx: number;
    readonly ry: number;
    /** Major-axis rotation in degrees; defaults to `0`. */
    readonly rotation?: number;
    /** Start parameter in degrees; defaults to `0`. */
    readonly startAngle?: number;
    /** End parameter in degrees; defaults to `360`. */
    readonly endAngle?: number;
}

/** Arguments of `addPolyline()`. */
export interface AddPolylineRequest {
    readonly layer: LayerId;
    readonly points: readonly Point[];
    readonly closed?: boolean;
}

/** Arguments of `addText()`. */
export interface AddTextRequest {
    readonly layer: LayerId;
    readonly at: Point;
    readonly height: number;
    readonly content: string;
}

/** Arguments of `addDimension()`. */
export interface AddDimensionRequest {
    readonly layer: LayerId;
    readonly from: Point;
    readonly to: Point;
    readonly offset: number;
    readonly text?: string;
}

/** Arguments of `addRadialDimension()`. */
export interface AddRadialDimensionRequest {
    readonly layer: LayerId;
    readonly center: Point;
    readonly radius: number;
    /** Degrees of the measured point from the positive X axis. */
    readonly angle: number;
    /** Signed distance from the measured point to the label; defaults to `0`. */
    readonly offset?: number;
    /** Measure the diameter instead of the radius; defaults to `false`. */
    readonly diameter?: boolean;
    readonly text?: string;
}

/** Arguments of `addOrdinateDimension()`. */
export interface AddOrdinateDimensionRequest {
    readonly layer: LayerId;
    readonly origin: Point;
    readonly feature: Point;
    /** Axis the dimension measures. */
    readonly axis: 'x' | 'y';
    /** Signed perpendicular distance of the dimension line from the origin; defaults to `0`. */
    readonly offset?: number;
    readonly text?: string;
}

/** Arguments of `addHatch()`. */
export interface AddHatchRequest {
    readonly layer: LayerId;
    /** At least three boundary vertices; the closing edge is implicit. */
    readonly points: readonly Point[];
    /** Pattern name; defaults to `SOLID`. */
    readonly pattern?: string;
}

/** Result of an entity mutation. Carries the refreshed drawing summary. */
export interface EntityChange {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly entity: Entity;
}

/** Result of `deleteEntity()`. */
export interface EntityDeletion {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly deleted: EntityId;
}

/** Result of `listEntities()`. */
export interface EntityList {
    readonly revision: number;
    readonly entities: readonly Entity[];
}

/** Axis-aligned rectangular extent in drawing units. */
export interface Bounds {
    readonly minX: number;
    readonly minY: number;
    readonly maxX: number;
    readonly maxY: number;
}

/** Selection a transform applies to. */
export type TransformTarget =
    | { readonly ids: readonly EntityId[] }
    | { readonly layer: LayerId }
    | { readonly all: true };

/** One affine operation of `transform()`. */
export type TransformOperation =
    | { readonly op: 'translate'; readonly dx: number; readonly dy: number }
    | { readonly op: 'rotate'; readonly angle: number; readonly cx: number; readonly cy: number }
    | { readonly op: 'scale'; readonly factor: number; readonly cx: number; readonly cy: number }
    | { readonly op: 'mirror'; readonly axis: 'x' | 'y'; readonly at: number };

/** Arguments of `transform()`. */
export interface TransformRequest {
    /** Sheets are not modelled; a non-empty value is rejected. */
    readonly sheet?: string;
    readonly target: TransformTarget;
    readonly operation: TransformOperation;
}

/** Result of `transform()`. `bounds` covers the whole drawing after the edit. */
export interface TransformResult {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly affected: number;
    readonly bounds: Bounds;
}

/** Arguments of `copy()`. */
export interface CopyRequest {
    readonly ids: readonly EntityId[];
    /** X offset of each successive copy. */
    readonly dx: number;
    /** Y offset of each successive copy. */
    readonly dy: number;
    /** Number of copies, 1-10000. */
    readonly count: number;
}

/** Result of `copy()`. */
export interface CopyResult {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly created: readonly EntityId[];
    readonly count: number;
}

/** Grid or polar placement of an `array()`. */
export type ArrayPlacement =
    | {
          readonly mode: 'rectangular';
          /** Rows including the source row, 1-10000. */
          readonly rows: number;
          /** Columns including the source column, 1-10000. */
          readonly columns: number;
          /** Vertical distance between adjacent rows. */
          readonly rowSpacing: number;
          /** Horizontal distance between adjacent columns. */
          readonly columnSpacing: number;
      }
    | {
          readonly mode: 'polar';
          readonly center: Point;
          /** Items including the source, 2-10000. */
          readonly count: number;
          /** Degrees; copies sit counter-clockwise at `totalAngle * step / count`. */
          readonly totalAngle: number;
      };

/**
 * Arguments of `array()`.
 *
 * The source stays at its own position, so `rows * columns - 1` rectangular
 * copies or `count - 1` polar copies are created, each with a fresh id.
 */
export interface ArrayRequest {
    readonly ids: readonly EntityId[];
    readonly placement: ArrayPlacement;
}

/** Result of `array()`. */
export interface ArrayResult {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly created: readonly EntityId[];
    readonly count: number;
}

/**
 * Result of `explode()`.
 *
 * The source polyline is replaced, at its list position, by one line per edge.
 */
export interface ExplodeResult {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly entity: EntityId;
    readonly created: readonly EntityId[];
    readonly count: number;
}

/** Arguments of `offset()`. */
export interface OffsetRequest {
    readonly entityId: EntityId;
    /** Positive grows a circle or arc and offsets left; negative shrinks or offsets right. */
    readonly distance: number;
}

/**
 * Result of `offset()`.
 *
 * The source entity stays in place and a new circle, arc or polyline is
 * appended on the same layer.
 */
export interface OffsetResult {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly entity: Entity;
    readonly source: EntityId;
}

/** Result of `fillet()` or `chamfer()`. Both source lines are trimmed. */
export interface CornerResult {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly entity: Entity;
    readonly sources: readonly EntityId[];
}

/**
 * Result of `measure()`.
 *
 * `length` sums line segments, polyline edges and arc lengths; ellipse length
 * is sampled at five-degree steps. `area` sums the shoelace area of closed
 * polylines, full circles and full ellipses; partial arcs and ellipses do not
 * contribute area.
 */
export interface MeasureResult {
    readonly revision: number;
    readonly bounds: Bounds;
    readonly width: number;
    readonly height: number;
    readonly entities: number;
    readonly length: number;
    readonly area: number;
}

/** One entity measured by `measureEntities()`. */
export interface EntityMeasurement {
    readonly id: EntityId;
    readonly layer: LayerId;
    readonly kind: EntityKindName;
    /** Whether the entity encloses an area, so `length` is its perimeter. */
    readonly closed: boolean;
    /** Path length, or perimeter for a closed entity. */
    readonly length: number;
    /** Enclosed area; zero for open curves and annotations. */
    readonly area: number;
}

/** Result of `measureEntities()`. */
export interface EntityMeasureResult {
    readonly revision: number;
    readonly units: 'mm' | 'inch';
    readonly entities: readonly EntityMeasurement[];
    readonly totals: {
        readonly entities: number;
        readonly length: number;
        readonly area: number;
    };
}

/** One layer's share of a drawing's statistics. */
export interface LayerStats {
    readonly id: LayerId;
    readonly name: string;
    readonly visible: boolean;
    readonly locked: boolean;
    readonly entityCount: number;
    readonly length: number;
    readonly area: number;
}

/**
 * Result of `stats()`.
 *
 * Length and area follow the same rules as `measure()`: analytic lines, arcs
 * and circles, five-degree ellipse sampling, and shoelace area only for closed
 * polylines, full circles and full ellipses. Annotations contribute nothing.
 */
export interface DrawingStats {
    readonly revision: number;
    readonly units: 'mm' | 'inch';
    readonly layers: readonly LayerStats[];
    readonly totals: {
        readonly entities: number;
        readonly length: number;
        readonly area: number;
    };
}

/** Arguments of `moveEntity()`. Exactly one of the offset and `to` forms. */
export interface MoveEntityRequest {
    readonly entityId: EntityId;
    /** Horizontal offset in drawing units. */
    readonly dx?: number;
    /** Vertical offset in drawing units. */
    readonly dy?: number;
    /** Absolute target for the entity anchor; cannot be combined with `dx`/`dy`. */
    readonly to?: Point;
}

/** Arguments of `updateEntity()`. Text takes `content`/`height`; a circle takes `radius`. */
export interface UpdateEntityRequest {
    readonly entityId: EntityId;
    readonly content?: string;
    readonly height?: number;
    readonly radius?: number;
}

/** Arguments of `findText()`. */
export interface FindTextRequest {
    /** Non-empty substring to look for. */
    readonly pattern: string;
    /** Match case exactly; defaults to false, which is ASCII case-insensitive. */
    readonly matchCase?: boolean;
    /** Restrict the search to one layer id. */
    readonly layer?: LayerId;
}

/** One text entity matched by `findText()`. */
export interface TextMatch {
    readonly id: EntityId;
    readonly layer: LayerId;
    readonly content: string;
    readonly at: Point;
}

/** Result of `findText()`. The query never changes the drawing. */
export interface TextMatchResult {
    readonly revision: number;
    readonly pattern: string;
    readonly matchCase: boolean;
    readonly layer: string | null;
    readonly count: number;
    readonly matches: readonly TextMatch[];
}

/** Arguments of `replaceText()`. */
export interface ReplaceTextRequest {
    /** Non-empty substring to replace. */
    readonly pattern: string;
    /** Replacement text; an empty string deletes the matched substrings. */
    readonly replacement: string;
    /** Match case exactly; defaults to false, which is ASCII case-insensitive. */
    readonly matchCase?: boolean;
    /** Restrict the edit to one layer id. */
    readonly layer?: LayerId;
}

/**
 * Result of `replaceText()`.
 *
 * `count` is the number of changed text entities and `occurrences` the number
 * of replaced substrings. A match on a locked layer refuses the whole edit.
 */
export interface TextReplaceResult {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly pattern: string;
    readonly replacement: string;
    readonly matchCase: boolean;
    readonly count: number;
    readonly occurrences: number;
}

/** Result of `undo()` and `redo()`. */
export interface HistoryResult {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly canUndo: boolean;
    readonly canRedo: boolean;
}

/** Arguments of `addLayer()`. */
export interface AddLayerRequest {
    readonly name: string;
    /** `#rgb` or `#rrggbb`; defaults to `#d0d7de`. */
    readonly color?: string;
}

/**
 * Arguments of `updateLayer()`.
 *
 * `name` also becomes the layer id, and every entity reference is rewritten in
 * the same mutation.
 */
export interface UpdateLayerRequest {
    readonly layerId: LayerId;
    readonly name?: string;
    readonly color?: string;
    readonly visible?: boolean;
    readonly locked?: boolean;
}

/** Result of a layer mutation. Carries the refreshed drawing summary. */
export interface LayerChange {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly layer: LayerSummary;
}

/** Arguments of `mergeLayer()`. */
export interface MergeLayerRequest {
    /** Layer whose entities move to `to`. It is removed after the merge. */
    readonly from: LayerId;
    /** Layer that receives the entities. */
    readonly to: LayerId;
}

/** Result of `mergeLayer()`. */
export interface LayerMerge {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly from: LayerId;
    readonly to: LayerId;
    /** Number of entities whose layer reference changed. */
    readonly moved: number;
}

/** Arguments of `convertUnits()`. */
export interface ConvertUnitsRequest {
    /** Unit the drawing converts to. */
    readonly to: 'mm' | 'inch';
}

/** Result of `convertUnits()`. */
export interface UnitsConversion {
    readonly revision: number;
    readonly drawing: Drawing;
    readonly from: 'mm' | 'inch';
    readonly to: 'mm' | 'inch';
    /** Multiplier applied to every length; `1` for a no-op conversion. */
    readonly factor: number;
    /** False when the drawing already used the requested unit. */
    readonly converted: boolean;
}

/** Arguments of `OpenCAD.importDxf()`. */
export interface ImportDxfRequest {
    /** Path of the ASCII DXF file. */
    readonly source: string;
    /** Title of the new drawing; defaults to the source file stem. */
    readonly title?: string;
}

/** One item the DXF reader assumed, approximated or skipped. */
export interface ImportLoss {
    readonly code: string;
    /** `notice` keeps the geometry, `warning` drops content. */
    readonly severity: 'notice' | 'warning';
    readonly detail: string;
}

/** Result of `OpenCAD.importDxf()`. */
export interface ImportDxfResult {
    readonly handle: number;
    readonly source: string;
    readonly drawing: Drawing;
    readonly imported: { readonly entities: number; readonly layers: number };
    readonly losses: readonly ImportLoss[];
}

/** Options of `OpenCAD.inspect()`. */
export interface InspectOptions {
    readonly checks?: readonly InspectCheck[];
}

/** One finding of an inspection run. */
export interface InspectIssue {
    readonly code: string;
    readonly severity: 'info' | 'warning' | 'error';
    readonly revision: number;
    readonly layerId: LayerId | null;
    readonly objectId: EntityId | null;
    readonly recoverable: boolean;
    readonly message: string;
}

/** Result of `OpenCAD.inspect()`. Findings only, never a quality score. */
export interface InspectReport {
    readonly revision: number;
    readonly checkerVersion: string;
    readonly issues: readonly InspectIssue[];
    readonly passed: boolean;
}

/** Options of `save()` (project file, not an exchange format). */
export interface SaveOptions {
    readonly target?: string;
    readonly overwrite?: boolean;
}

/** Result of `save()`. */
export interface SaveReport {
    readonly path: string;
    readonly revision: number;
    readonly bytes: number;
}

/** Options of `export()`. */
export interface ExportOptions {
    readonly format: ExportFormat;
    readonly target: string;
    readonly overwrite?: boolean;
}

/** One thing a projection could not keep. */
export interface ExportLoss {
    readonly code: string;
    readonly severity: 'warning' | 'error';
    readonly layerId?: LayerId;
    readonly entityId?: EntityId;
    readonly message: string;
}

/** Result of `export()`. Reports what the target actually kept. */
export interface ExportReport {
    readonly revision: number;
    readonly format: ExportFormat;
    readonly target: string;
    /** Absolute target path; the same value as `target`. */
    readonly path?: string;
    /** Number of bytes written. */
    readonly bytes?: number;
    /** Triangles written by an `stl` or `obj` projection. */
    readonly triangles?: number;
    /** Planar faces written by a `step` projection. */
    readonly faces?: number;
    readonly losses: readonly ExportLoss[];
    /** False for projections such as PDF that cannot be reopened for editing. */
    readonly editable: boolean;
}
