/**
 * Orthogonal native operations.
 *
 * One function performs one operation and returns data; argument parsing, help
 * text and exit codes belong to `cli.ts` and to the session surface. Nothing
 * here knows about a command line. Every function forwards to the optional
 * native addon and decodes one JSON string.
 */

import { NotImplementedError, OpenCADError, errorCodes, type OpenCADErrorCode } from './errors.js';
import { nativeBinding } from './native.js';
import type {
    AddArcRequest,
    AddCircleRequest,
    AddDimensionRequest,
    AddDimensionStyleRequest,
    AddEllipseRequest,
    AddHatchRequest,
    AddLayerRequest,
    AddLineRequest,
    AddLinetypeRequest,
    AddOrdinateDimensionRequest,
    AddPointRequest,
    AddPolylineRequest,
    AddRadialDimensionRequest,
    AddTextRequest,
    AddTextStyleRequest,
    ArrayRequest,
    ArrayResult,
    AssignEntityLayerRequest,
    ConvertUnitsRequest,
    CopyRequest,
    CopyResult,
    CreateGroupRequest,
    CornerResult,
    CreateLayoutRequest,
    DimensionStyleChange,
    DimensionStyleDeletion,
    DimensionStyleList,
    Drawing,
    DrawingPropertiesChange,
    DrawingPropertiesResult,
    DrawOrderAction,
    DrawOrderResult,
    EntityChange,
    EntityDeletion,
    EntityGroup,
    EntityLayerChange,
    EntityList,
    EntityMeasureResult,
    ExplodeResult,
    ExportFormat,
    ExportReport,
    FindTextRequest,
    GroupChange,
    GroupDeletion,
    GroupList,
    GroupMembersRequest,
    HistoryResult,
    ImportDxfRequest,
    ImportDxfResult,
    InspectCheck,
    InspectReport,
    LayerChange,
    LayerDeletion,
    LayerMerge,
    DrawingStats,
    LayoutChange,
    LayoutDeletion,
    LayoutList,
    LinetypeChange,
    LinetypeDeletion,
    LinetypeList,
    MeasureResult,
    MergeLayerRequest,
    MoveEntityRequest,
    OffsetRequest,
    OffsetResult,
    OpenCADCapabilities,
    Point,
    PointMeasurement,
    ReplaceTextRequest,
    SaveReport,
    SelectionRequest,
    SelectionResult,
    SnapRequest,
    SnapResult,
    TextStyleChange,
    TextStyleDeletion,
    TextStyleList,
    TextMatchResult,
    TextReplaceResult,
    TransformRequest,
    TransformResult,
    UcsRequest,
    UcsResult,
    UnitsConversion,
    UpdateArcRequest,
    UpdateDimensionStyleRequest,
    UpdateEntityRequest,
    UpdateLayerRequest,
    UpdateLayoutRequest,
    UpdateLinetypeRequest,
    UpdatePolylineRequest,
    UpdatePropertiesRequest,
    UpdateTextStyleRequest,
} from './types.js';

/** Result of creating or opening a drawing. */
export interface DrawingHandle {
    readonly handle: number;
    readonly drawing: Drawing;
}

/** Arguments of {@link create}. */
export interface CreateDrawingRequest {
    readonly title?: string;
    readonly units?: 'mm' | 'inch';
    readonly template?: string;
}

const knownCodes = new Set<string>(errorCodes);

function binding(feature: string) {
    const native = nativeBinding();
    if (!native) {
        throw new NotImplementedError(feature);
    }
    return native;
}

function decode<T>(work: () => string): T {
    try {
        return JSON.parse(work()) as T;
    } catch (error) {
        throw translate(error);
    }
}

function translate(error: unknown): OpenCADError {
    if (error instanceof OpenCADError) {
        return error;
    }
    const message = error instanceof Error ? error.message : String(error);
    const separator = message.indexOf(': ');
    const code = separator > 0 ? message.slice(0, separator) : '';
    if (knownCodes.has(code)) {
        return new OpenCADError(code as OpenCADErrorCode, message.slice(separator + 2), { details: { feature: code } });
    }
    return new OpenCADError('format/invalid', message, { cause: error });
}

/** Reads the capability matrix of the installed addon. */
export function capabilities(): OpenCADCapabilities {
    return decode<OpenCADCapabilities>(() => binding('capabilities').capabilities());
}

/** Creates a drawing and returns its handle. */
export function create(request: CreateDrawingRequest): DrawingHandle {
    return decode<DrawingHandle>(() =>
        binding('create').drawing_create(JSON.stringify({ title: request.title, units: request.units, template: request.template })),
    );
}

/** Opens a project file and returns its handle. */
export function open(path: string): DrawingHandle {
    return decode<DrawingHandle>(() => binding('open').drawing_open(path));
}

/** Writes one open drawing to a project path. */
export function save(handle: number, target: string, overwrite: boolean): SaveReport {
    return decode<SaveReport>(() => binding('save').drawing_save(handle, target, overwrite));
}

/** Runs the requested checks on one open drawing. */
export function inspect(handle: number, checks?: ReadonlyArray<InspectCheck>): InspectReport {
    return decode<InspectReport>(() =>
        checks === undefined
            ? binding('inspect').drawing_inspect(handle)
            : binding('inspect').drawing_inspect(handle, JSON.stringify(checks)),
    );
}

/** Projects one open drawing into an exchange format. */
export function exportDrawing(handle: number, format: ExportFormat, target: string): ExportReport {
    return decode<ExportReport>(() => binding('export').drawing_export(handle, format, target));
}

/** Adds a line to one layer. */
export function addLine(handle: number, request: AddLineRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addLine').drawing_add_line(handle, JSON.stringify({ layer: request.layer, from: request.from, to: request.to })),
    );
}

/** Adds a point entity to one layer. */
export function addPoint(handle: number, request: AddPointRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addPoint').drawing_add_point(handle, JSON.stringify({ layer: request.layer, at: request.at })),
    );
}

/** Adds a circle to one layer. */
export function addCircle(handle: number, request: AddCircleRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addCircle').drawing_add_circle(
            handle,
            JSON.stringify({ layer: request.layer, center: request.center, radius: request.radius }),
        ),
    );
}

/** Adds a circular arc to one layer. */
export function addArc(handle: number, request: AddArcRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addArc').drawing_add_arc(
            handle,
            JSON.stringify({
                layer: request.layer,
                center: request.center,
                radius: request.radius,
                start_angle: request.startAngle,
                end_angle: request.endAngle ?? request.startAngle + 360,
            }),
        ),
    );
}

/** Adds a full or partial ellipse to one layer. */
export function addEllipse(handle: number, request: AddEllipseRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addEllipse').drawing_add_ellipse(
            handle,
            JSON.stringify({
                layer: request.layer,
                center: request.center,
                rx: request.rx,
                ry: request.ry,
                rotation: request.rotation ?? 0,
                start_angle: request.startAngle ?? 0,
                end_angle: request.endAngle ?? 360,
            }),
        ),
    );
}

/** Adds a polyline to one layer. */
export function addPolyline(handle: number, request: AddPolylineRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addPolyline').drawing_add_polyline(
            handle,
            JSON.stringify({ layer: request.layer, points: request.points, closed: request.closed ?? false }),
        ),
    );
}

/** Adds single-line text to one layer. */
export function addText(handle: number, request: AddTextRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addText').drawing_add_text(
            handle,
            JSON.stringify({ layer: request.layer, at: request.at, height: request.height, content: request.content }),
        ),
    );
}

/** Adds a linear dimension to one layer. */
export function addDimension(handle: number, request: AddDimensionRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addDimension').drawing_add_dimension(
            handle,
            JSON.stringify({
                layer: request.layer,
                from: request.from,
                to: request.to,
                offset: request.offset,
                text: request.text,
            }),
        ),
    );
}

/** Adds a radial or diameter dimension to one layer. */
export function addRadialDimension(handle: number, request: AddRadialDimensionRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addRadialDimension').drawing_add_radial_dimension(
            handle,
            JSON.stringify({
                layer: request.layer,
                center: request.center,
                radius: request.radius,
                angle: request.angle,
                offset: request.offset ?? 0,
                diameter: request.diameter ?? false,
                text: request.text,
            }),
        ),
    );
}

/** Adds an ordinate dimension to one layer. */
export function addOrdinateDimension(handle: number, request: AddOrdinateDimensionRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addOrdinateDimension').drawing_add_ordinate_dimension(
            handle,
            JSON.stringify({
                layer: request.layer,
                origin: request.origin,
                feature: request.feature,
                axis: request.axis,
                offset: request.offset ?? 0,
                text: request.text,
            }),
        ),
    );
}

/** Adds a filled hatch bounded by one closed polygon. */
export function addHatch(handle: number, request: AddHatchRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.addHatch').drawing_add_hatch(
            handle,
            JSON.stringify({
                layer: request.layer,
                points: request.points,
                pattern: request.pattern,
            }),
        ),
    );
}

/** Adds a fillet arc between two lines and trims both lines. */
export function fillet(handle: number, first: string, second: string, radius: number): CornerResult {
    return decode<CornerResult>(() =>
        binding('drawing.fillet').drawing_fillet(handle, JSON.stringify({ first, second, radius })),
    );
}

/** Adds a chamfer line between two lines and trims both lines. */
export function chamfer(handle: number, first: string, second: string, distanceA: number, distanceB: number): CornerResult {
    return decode<CornerResult>(() =>
        binding('drawing.chamfer').drawing_chamfer(
            handle,
            JSON.stringify({ first, second, distanceA, distanceB }),
        ),
    );
}

/** Deletes one entity by id. */
export function deleteEntity(handle: number, entityId: string): EntityDeletion {
    return decode<EntityDeletion>(() => binding('drawing.deleteEntity').drawing_delete_entity(handle, entityId));
}

/** Lists every entity of one open drawing. */
export function listEntities(handle: number): EntityList {
    return decode<EntityList>(() => binding('drawing.listEntities').drawing_list_entities(handle));
}

/** Moves one entity within the paint order. */
export function reorderEntity(handle: number, entityId: string, action: DrawOrderAction): DrawOrderResult {
    return decode<DrawOrderResult>(() =>
        binding('drawing.reorderEntity').drawing_reorder_entity(handle, entityId, action),
    );
}

/** Lists the paper-space layouts of one open drawing. */
export function listLayouts(handle: number): LayoutList {
    return decode<LayoutList>(() => binding('drawing.listLayouts').drawing_list_layouts(handle));
}

/** Creates one paper-space layout. */
export function createLayout(handle: number, request: CreateLayoutRequest): LayoutChange {
    return decode<LayoutChange>(() =>
        binding('drawing.createLayout').drawing_create_layout(
            handle,
            JSON.stringify({
                name: request.name,
                width: request.width,
                height: request.height,
                scale: request.scale,
            }),
        ),
    );
}

/** Updates one paper-space layout. */
export function updateLayout(handle: number, request: UpdateLayoutRequest): LayoutChange {
    return decode<LayoutChange>(() =>
        binding('drawing.updateLayout').drawing_update_layout(
            handle,
            request.layoutId,
            JSON.stringify({
                name: request.name,
                width: request.width,
                height: request.height,
                scale: request.scale,
            }),
        ),
    );
}

/** Removes one paper-space layout. */
export function deleteLayout(handle: number, layoutId: string): LayoutDeletion {
    return decode<LayoutDeletion>(() => binding('drawing.deleteLayout').drawing_delete_layout(handle, layoutId));
}

/** Reads the title and descriptive properties of one open drawing. */
export function properties(handle: number): DrawingPropertiesResult {
    return decode<DrawingPropertiesResult>(() => binding('drawing.properties').drawing_properties(handle));
}

/** Updates the title and descriptive properties of one open drawing. */
export function updateProperties(handle: number, request: UpdatePropertiesRequest): DrawingPropertiesChange {
    return decode<DrawingPropertiesChange>(() =>
        binding('drawing.updateProperties').drawing_update_properties(
            handle,
            JSON.stringify({
                title: request.title,
                author: request.author,
                description: request.description,
                company: request.company,
                project: request.project,
                keywords: request.keywords,
            }),
        ),
    );
}

/** Restores the previous snapshot of one drawing. */
export function undo(handle: number): HistoryResult {
    return decode<HistoryResult>(() => binding('drawing.undo').drawing_undo(handle));
}

/** Reapplies the snapshot the last undo replaced. */
export function redo(handle: number): HistoryResult {
    return decode<HistoryResult>(() => binding('drawing.redo').drawing_redo(handle));
}

/** Adds one layer. */
export function addLayer(handle: number, request: AddLayerRequest): LayerChange {
    return decode<LayerChange>(() =>
        binding('drawing.addLayer').drawing_add_layer(handle, JSON.stringify({ name: request.name, color: request.color })),
    );
}

/** Updates one layer, rewriting entity references when the name changes. */
export function updateLayer(handle: number, request: UpdateLayerRequest): LayerChange {
    return decode<LayerChange>(() =>
        binding('drawing.updateLayer').drawing_update_layer(
            handle,
            JSON.stringify({
                layerId: request.layerId,
                name: request.name,
                color: request.color,
                visible: request.visible,
                locked: request.locked,
                frozen: request.frozen,
                plottable: request.plottable,
                linetype: request.linetype,
                lineweight: request.lineweight,
            }),
        ),
    );
}

/** Moves every entity of one layer onto another and drops the empty source. */
export function mergeLayer(handle: number, request: MergeLayerRequest): LayerMerge {
    return decode<LayerMerge>(() => binding('drawing.mergeLayer').drawing_merge_layer(handle, request.from, request.to));
}

/** Removes one empty layer. */
export function deleteLayer(handle: number, layerId: string): LayerDeletion {
    return decode<LayerDeletion>(() => binding('drawing.deleteLayer').drawing_delete_layer(handle, layerId));
}

/** Lists the dimension styles of one open drawing. */
export function listDimStyles(handle: number): DimensionStyleList {
    return decode<DimensionStyleList>(() => binding('drawing.listDimStyles').drawing_list_dim_styles(handle));
}

/** Adds one dimension style to an open drawing. */
export function addDimStyle(handle: number, request: AddDimensionStyleRequest): DimensionStyleChange {
    return decode<DimensionStyleChange>(() =>
        binding('drawing.addDimStyle').drawing_add_dim_style(
            handle,
            JSON.stringify({
                name: request.name,
                scale: request.scale,
                arrowSize: request.arrowSize,
                textHeight: request.textHeight,
                extension: request.extension,
                originOffset: request.originOffset,
                precision: request.precision,
            }),
        ),
    );
}

/** Updates one dimension style of an open drawing. */
export function updateDimStyle(handle: number, request: UpdateDimensionStyleRequest): DimensionStyleChange {
    return decode<DimensionStyleChange>(() =>
        binding('drawing.updateDimStyle').drawing_update_dim_style(
            handle,
            JSON.stringify({
                styleId: request.styleId,
                name: request.name,
                scale: request.scale,
                arrowSize: request.arrowSize,
                textHeight: request.textHeight,
                extension: request.extension,
                originOffset: request.originOffset,
                precision: request.precision,
            }),
        ),
    );
}

/** Removes one dimension style from an open drawing. */
export function deleteDimStyle(handle: number, styleId: string): DimensionStyleDeletion {
    return decode<DimensionStyleDeletion>(() => binding('drawing.deleteDimStyle').drawing_delete_dim_style(handle, styleId));
}

/** Lists the text styles of one open drawing. */
export function listTextStyles(handle: number): TextStyleList {
    return decode<TextStyleList>(() => binding('drawing.listTextStyles').drawing_list_text_styles(handle));
}

/** Adds one text style to an open drawing. */
export function addTextStyle(handle: number, request: AddTextStyleRequest): TextStyleChange {
    return decode<TextStyleChange>(() =>
        binding('drawing.addTextStyle').drawing_add_text_style(
            handle,
            JSON.stringify({
                name: request.name,
                font: request.font,
                height: request.height,
                widthFactor: request.widthFactor,
                oblique: request.oblique,
            }),
        ),
    );
}

/** Updates one text style of an open drawing. */
export function updateTextStyle(handle: number, request: UpdateTextStyleRequest): TextStyleChange {
    return decode<TextStyleChange>(() =>
        binding('drawing.updateTextStyle').drawing_update_text_style(
            handle,
            JSON.stringify({
                styleId: request.styleId,
                name: request.name,
                font: request.font,
                height: request.height,
                widthFactor: request.widthFactor,
                oblique: request.oblique,
            }),
        ),
    );
}

/** Removes one text style from an open drawing. */
export function deleteTextStyle(handle: number, styleId: string): TextStyleDeletion {
    return decode<TextStyleDeletion>(() => binding('drawing.deleteTextStyle').drawing_delete_text_style(handle, styleId));
}

/** Lists the line types of one open drawing. */
export function listLinetypes(handle: number): LinetypeList {
    return decode<LinetypeList>(() => binding('drawing.listLinetypes').drawing_list_linetypes(handle));
}

/** Adds one line type to an open drawing. */
export function addLinetype(handle: number, request: AddLinetypeRequest): LinetypeChange {
    return decode<LinetypeChange>(() =>
        binding('drawing.addLinetype').drawing_add_linetype(
            handle,
            JSON.stringify({
                name: request.name,
                description: request.description,
                pattern: request.pattern,
            }),
        ),
    );
}

/** Updates one line type, rewriting layer references when the name changes. */
export function updateLinetype(handle: number, request: UpdateLinetypeRequest): LinetypeChange {
    return decode<LinetypeChange>(() =>
        binding('drawing.updateLinetype').drawing_update_linetype(
            handle,
            JSON.stringify({
                linetypeId: request.linetypeId,
                name: request.name,
                description: request.description,
                pattern: request.pattern,
            }),
        ),
    );
}

/** Removes one line type from an open drawing. */
export function deleteLinetype(handle: number, linetypeId: string): LinetypeDeletion {
    return decode<LinetypeDeletion>(() => binding('drawing.deleteLinetype').drawing_delete_linetype(handle, linetypeId));
}

/** Lists the entity groups of one open drawing. */
export function listGroups(handle: number): GroupList {
    return decode<GroupList>(() => binding('drawing.listGroups').drawing_list_groups(handle));
}

/** Creates one entity group from existing entities. */
export function createGroup(handle: number, request: CreateGroupRequest): GroupChange {
    return decode<GroupChange>(() =>
        binding('drawing.createGroup').drawing_create_group(
            handle,
            JSON.stringify({ name: request.name, entities: request.entities }),
        ),
    );
}

/** Removes one entity group. */
export function deleteGroup(handle: number, groupId: string): GroupDeletion {
    return decode<GroupDeletion>(() => binding('drawing.deleteGroup').drawing_delete_group(handle, groupId));
}

/** Adds existing entities to one group. */
export function addGroupMembers(handle: number, request: GroupMembersRequest): GroupChange {
    return decode<GroupChange>(() =>
        binding('drawing.addGroupMembers').drawing_add_group_members(
            handle,
            JSON.stringify({ groupId: request.groupId, entities: request.entities }),
        ),
    );
}

/** Removes entities from one group. */
export function removeGroupMembers(handle: number, request: GroupMembersRequest): GroupChange {
    return decode<GroupChange>(() =>
        binding('drawing.removeGroupMembers').drawing_remove_group_members(
            handle,
            JSON.stringify({ groupId: request.groupId, entities: request.entities }),
        ),
    );
}

/** Selects entities with a rectangular window or crossing box. */
export function select(handle: number, request: SelectionRequest): SelectionResult {
    return decode<SelectionResult>(() =>
        binding('drawing.select').drawing_select(
            handle,
            JSON.stringify({ window: request.window, mode: request.mode ?? 'window', layer: request.layer }),
        ),
    );
}

/** Computes object snap candidates without changing the drawing. */
export function snap(handle: number, request: SnapRequest): SnapResult {
    return decode<SnapResult>(() =>
        binding('drawing.snap').drawing_snap(
            handle,
            JSON.stringify({
                modes: request.modes,
                point: request.point,
                tolerance: request.tolerance,
                layer: request.layer,
                ids: request.ids,
            }),
        ),
    );
}

/** Converts one open drawing between millimetres and inches. */
export function convertUnits(handle: number, request: ConvertUnitsRequest): UnitsConversion {
    return decode<UnitsConversion>(() => binding('drawing.convertUnits').drawing_convert_units(handle, request.to));
}

/** Moves one entity by an offset or to an absolute anchor. */
export function moveEntity(handle: number, request: MoveEntityRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.moveEntity').drawing_move_entity(
            handle,
            JSON.stringify({ entityId: request.entityId, dx: request.dx, dy: request.dy, to: request.to }),
        ),
    );
}

/** Updates the payload of one text, circle or arc entity. */
export function updateEntity(handle: number, request: UpdateEntityRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.updateEntity').drawing_update_entity(
            handle,
            JSON.stringify({
                entityId: request.entityId,
                content: request.content,
                height: request.height,
                radius: request.radius,
            }),
        ),
    );
}

/** Edits the vertices or closed flag of one polyline. */
export function updatePolyline(handle: number, request: UpdatePolylineRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.updatePolyline').drawing_update_polyline(
            handle,
            JSON.stringify({
                entityId: request.entityId,
                points: request.points,
                closed: request.closed,
                insertAt: request.insertAt,
                insertPoint: request.insertPoint,
                removeAt: request.removeAt,
            }),
        ),
    );
}

/** Updates the radius or sweep angles of one arc. */
export function updateArc(handle: number, request: UpdateArcRequest): EntityChange {
    return decode<EntityChange>(() =>
        binding('drawing.updateArc').drawing_update_arc(
            handle,
            JSON.stringify({
                entityId: request.entityId,
                radius: request.radius,
                startAngle: request.startAngle,
                endAngle: request.endAngle,
            }),
        ),
    );
}

/** Moves one entity onto another layer without changing its geometry. */
export function assignEntityLayer(handle: number, request: AssignEntityLayerRequest): EntityLayerChange {
    return decode<EntityLayerChange>(() =>
        binding('drawing.assignEntityLayer').drawing_assign_entity_layer(
            handle,
            JSON.stringify({ entityId: request.entityId, layer: request.layer }),
        ),
    );
}

/** Finds text entities whose content contains a pattern. */
export function findText(handle: number, request: FindTextRequest): TextMatchResult {
    return decode<TextMatchResult>(() =>
        binding('drawing.findText').drawing_find_text(
            handle,
            JSON.stringify({
                pattern: request.pattern,
                matchCase: request.matchCase ?? false,
                layer: request.layer,
            }),
        ),
    );
}

/** Replaces a pattern in every matching text entity. */
export function replaceText(handle: number, request: ReplaceTextRequest): TextReplaceResult {
    return decode<TextReplaceResult>(() =>
        binding('drawing.replaceText').drawing_replace_text(
            handle,
            JSON.stringify({
                pattern: request.pattern,
                replacement: request.replacement,
                matchCase: request.matchCase ?? false,
                layer: request.layer,
            }),
        ),
    );
}

/** Applies one affine transform to a selection. */
export function transform(handle: number, request: TransformRequest): TransformResult {
    return decode<TransformResult>(() =>
        binding('drawing.transform').drawing_transform(handle, JSON.stringify(request)),
    );
}

/** Duplicates entities with a cumulative offset. */
export function copy(handle: number, request: CopyRequest): CopyResult {
    return decode<CopyResult>(() =>
        binding('drawing.copy').drawing_copy(
            handle,
            JSON.stringify({ ids: request.ids, dx: request.dx, dy: request.dy, count: request.count }),
        ),
    );
}

/** Duplicates entities into a rectangular or polar array. */
export function array(handle: number, request: ArrayRequest): ArrayResult {
    const placement = request.placement;
    const payload =
        placement.mode === 'rectangular'
            ? {
                  ids: request.ids,
                  mode: 'rectangular',
                  rows: placement.rows,
                  columns: placement.columns,
                  row_spacing: placement.rowSpacing,
                  column_spacing: placement.columnSpacing,
              }
            : {
                  ids: request.ids,
                  mode: 'polar',
                  center: placement.center,
                  count: placement.count,
                  total_angle: placement.totalAngle,
              };
    return decode<ArrayResult>(() => binding('drawing.array').drawing_array(handle, JSON.stringify(payload)));
}

/** Replaces one polyline with independent line entities. */
export function explode(handle: number, entityId: string): ExplodeResult {
    return decode<ExplodeResult>(() => binding('drawing.explode').drawing_explode(handle, entityId));
}

/** Offsets a circle, arc, polyline or line into a new entity on the same layer. */
export function offset(handle: number, request: OffsetRequest): OffsetResult {
    return decode<OffsetResult>(() =>
        binding('drawing.offset').drawing_offset(handle, JSON.stringify({ entityId: request.entityId, distance: request.distance })),
    );
}

/** Measures bounds, path length and enclosed area. */
export function measure(handle: number, sheet?: string): MeasureResult {
    return decode<MeasureResult>(() =>
        sheet === undefined ? binding('drawing.measure').drawing_measure(handle) : binding('drawing.measure').drawing_measure(handle, sheet),
    );
}

/** Measures the path length and enclosed area of one or more entities. */
export function measureEntities(handle: number, ids: readonly string[]): EntityMeasureResult {
    return decode<EntityMeasureResult>(() =>
        binding('drawing.measureEntities').drawing_measure_entities(handle, JSON.stringify(ids)),
    );
}

/** Measures the distance between two points or the angle at three points. */
export function measurePoints(handle: number, points: readonly Point[]): PointMeasurement {
    return decode<PointMeasurement>(() => binding('drawing.measurePoints').drawing_measure_points(handle, JSON.stringify(points)));
}

/** Converts points and entities between world and user coordinates. */
export function ucs(handle: number, request: UcsRequest): UcsResult {
    return decode<UcsResult>(() =>
        binding('drawing.ucs').drawing_ucs(
            handle,
            JSON.stringify({
                origin: request.origin ?? [0, 0],
                rotation: request.rotation ?? 0,
                direction: request.direction ?? 'to-world',
                points: request.points ?? [],
                ids: request.ids ?? [],
            }),
        ),
    );
}

/** Summarises entity count, path length and enclosed area per layer. */
export function stats(handle: number): DrawingStats {
    return decode<DrawingStats>(() => binding('drawing.stats').drawing_stats(handle));
}

/** Imports the declared ASCII DXF subset into a new drawing handle. */
export function importDxf(request: ImportDxfRequest): ImportDxfResult {
    return decode<ImportDxfResult>(() =>
        binding('drawing.importDxf').drawing_import_dxf(JSON.stringify({ source: request.source, title: request.title })),
    );
}

/** Releases one drawing handle. */
export function close(handle: number): void {
    binding('drawing.close').drawing_close(handle);
}
