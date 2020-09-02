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
    AddEllipseRequest,
    AddLayerRequest,
    AddLineRequest,
    AddPolylineRequest,
    AddTextRequest,
    CopyRequest,
    CopyResult,
    Drawing,
    EntityChange,
    EntityDeletion,
    EntityList,
    ExportFormat,
    ExportReport,
    HistoryResult,
    ImportDxfRequest,
    ImportDxfResult,
    InspectCheck,
    InspectReport,
    LayerChange,
    MeasureResult,
    MoveEntityRequest,
    OpenCADCapabilities,
    SaveReport,
    TransformRequest,
    TransformResult,
    UpdateEntityRequest,
    UpdateLayerRequest,
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

/** Deletes one entity by id. */
export function deleteEntity(handle: number, entityId: string): EntityDeletion {
    return decode<EntityDeletion>(() => binding('drawing.deleteEntity').drawing_delete_entity(handle, entityId));
}

/** Lists every entity of one open drawing. */
export function listEntities(handle: number): EntityList {
    return decode<EntityList>(() => binding('drawing.listEntities').drawing_list_entities(handle));
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
            }),
        ),
    );
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

/** Measures bounds, path length and enclosed area. */
export function measure(handle: number, sheet?: string): MeasureResult {
    return decode<MeasureResult>(() =>
        sheet === undefined ? binding('drawing.measure').drawing_measure(handle) : binding('drawing.measure').drawing_measure(handle, sheet),
    );
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
