/**
 * Handle-backed drawing document.
 *
 * `OpenCAD.create()` and `OpenCAD.open()` return one of these. The summary
 * stays in sync with every mutation, and every method forwards to the native
 * store through the handle it owns.
 *
 * In a `history: 'sidecar'` session the document also keeps
 * `<file>.history.json` up to date: every mutation is written back to the
 * project file and the replaced document is recorded, so undo and redo work
 * across processes. The default memory session leaves history to the handle.
 */

import { OpenCADError } from './errors.js';
import * as service from './service.js';
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
    CopyRequest,
    CopyResult,
    ConvertUnitsRequest,
    CreateGroupRequest,
    CreateLayoutRequest,
    DimensionStyle,
    DimensionStyleDeletion,
    Drawing,
    DrawingProperties,
    DrawOrderAction,
    DrawOrderResult,
    Entity,
    EntityDeletion,
    EntityGroup,
    EntityId,
    EntityList,
    EntityMeasureResult,
    DrawingStats,
    ExplodeResult,
    ExportOptions,
    ExportReport,
    FindTextRequest,
    GroupDeletion,
    GroupMembersRequest,
    HistoryMode,
    HistoryResult,
    ImportLoss,
    InspectOptions,
    InspectReport,
    LayerDeletion,
    LayerMerge,
    LayerSummary,
    Layout,
    LayoutDeletion,
    Linetype,
    LinetypeDeletion,
    OffsetRequest,
    OffsetResult,
    MeasureResult,
    Point,
    PointMeasurement,
    MergeLayerRequest,
    MoveEntityRequest,
    ReplaceTextRequest,
    SaveOptions,
    SaveReport,
    SelectionRequest,
    SelectionResult,
    SnapRequest,
    SnapResult,
    TextStyle,
    TextStyleDeletion,
    TextMatchResult,
    TextReplaceResult,
    TransformRequest,
    TransformResult,
    UcsRequest,
    UcsResult,
    UnitsConversion,
    UpdateDimensionStyleRequest,
    UpdateEntityRequest,
    UpdateLayerRequest,
    UpdateLayoutRequest,
    UpdateLinetypeRequest,
    UpdatePolylineRequest,
    UpdatePropertiesRequest,
    UpdateTextStyleRequest,
} from './types.js';

type HistoryModule = typeof import('./history.js');

/** Options a document inherits from the session that opened it. */
export interface DrawingDocumentOptions {
    /** Undo/redo backing store. Defaults to `memory`. */
    readonly history?: HistoryMode;
    /** Project file this document was opened from, if any. */
    readonly sourcePath?: string;
    /** Sidecar helpers; present only in a `sidecar` session. */
    readonly historyModule?: HistoryModule;
    /** Import statistics when the document came from `importDxf()`. */
    readonly imported?: { readonly entities: number; readonly layers: number };
    /** Import losses reported by `importDxf()`. */
    readonly losses?: readonly ImportLoss[];
}

/** One open drawing. Owns exactly one native handle. */
export class DrawingDocument {
    private handleValue: number;
    private state: Drawing;
    private readonly historyMode: HistoryMode;
    private readonly historyModule: HistoryModule | undefined;
    private readonly importedCounts: { readonly entities: number; readonly layers: number } | undefined;
    private readonly importLosses: readonly ImportLoss[] | undefined;
    private sourcePath: string | undefined;
    private sidecar: import('./history.js').SidecarHistory | undefined;
    private closed = false;

    constructor(handle: number, drawing: Drawing, options: DrawingDocumentOptions = {}) {
        this.handleValue = handle;
        this.state = drawing;
        this.historyMode = options.history ?? 'memory';
        this.historyModule = options.historyModule;
        this.importedCounts = options.imported;
        this.importLosses = options.losses;
        this.sourcePath = options.sourcePath;
        this.sidecar =
            this.historyMode === 'sidecar' && this.historyModule !== undefined && this.sourcePath !== undefined
                ? new this.historyModule.SidecarHistory(this.sourcePath)
                : undefined;
    }

    /** Native handle of this document. */
    get handle(): number {
        return this.handleValue;
    }

    /** Drawing identity. */
    get id(): string {
        return this.state.id;
    }

    /** Committed revision. */
    get revision(): number {
        return this.state.revision;
    }

    /** Title, or `null` when the drawing is untitled. */
    get title(): string | null {
        return this.state.title;
    }

    /** Declared unit. */
    get units(): 'mm' | 'inch' {
        return this.state.units;
    }

    /** Descriptive title-block properties. */
    get properties(): DrawingProperties {
        return this.state.properties;
    }

    /** Layers in paint order. */
    get layers(): readonly LayerSummary[] {
        return this.state.layers;
    }

    /** Named dimension styles of this document. */
    get dimStyles(): readonly DimensionStyle[] {
        return this.state.dimStyles;
    }

    /** Named text styles of this document. */
    get textStyles(): readonly TextStyle[] {
        return this.state.textStyles;
    }

    /** Named line types of this document. */
    get linetypes(): readonly Linetype[] {
        return this.state.linetypes;
    }

    /** Number of entities. */
    get entities(): number {
        return this.state.entities;
    }

    /** Whether an earlier snapshot can be restored. */
    get canUndo(): boolean {
        if (this.sidecar !== undefined) {
            return this.sidecar.counts().undo > 0;
        }
        return this.state.canUndo;
    }

    /** Whether an undone snapshot can be reapplied. */
    get canRedo(): boolean {
        if (this.sidecar !== undefined) {
            return this.sidecar.counts().redo > 0;
        }
        return this.state.canRedo;
    }

    /** Whether {@link close} has run. */
    get isClosed(): boolean {
        return this.closed;
    }

    /** Import statistics when this document came from `importDxf()`. */
    get imported(): { readonly entities: number; readonly layers: number } | undefined {
        return this.importedCounts;
    }

    /** Import losses when this document came from `importDxf()`. */
    get losses(): readonly ImportLoss[] | undefined {
        return this.importLosses;
    }

    /** Latest summary returned by the native store. */
    summary(): Drawing {
        return this.state;
    }

    /** Latest summary, also available under the `drawing` field name. */
    get drawing(): Drawing {
        return this.state;
    }

    /** Updates the title and descriptive properties and refreshes the summary. */
    updateProperties(request: UpdatePropertiesRequest): DrawingProperties {
        let properties: DrawingProperties | undefined;
        this.edit(
            (handle) => service.updateProperties(handle, request),
            (result) => {
                this.state = result.drawing;
                properties = result.properties;
            },
        );
        return properties as DrawingProperties;
    }

    /** Runs the requested checks against this document. */
    inspect(options?: InspectOptions): InspectReport {
        this.assertOpen();
        return service.inspect(this.handle, options?.checks);
    }

    /**
     * Writes this document to a project path.
     *
     * `target` defaults to the file this document was opened from or last saved
     * to. In a sidecar session, saving to a new target adopts it as the source
     * path and drops the target's stale history.
     */
    save(options: SaveOptions = {}): SaveReport {
        this.assertOpen();
        const target = options.target ?? this.sourcePath;
        if (target === undefined) {
            throw new OpenCADError('session/argument-invalid', '`save()` needs a target before this document has ever been saved', {
                details: { feature: 'drawing.save' },
            });
        }
        const report = service.save(this.handle, target, options.overwrite ?? true);
        this.state = { ...this.state, revision: report.revision };
        if (this.historyMode === 'sidecar' && this.historyModule !== undefined) {
            if (this.sourcePath !== target) {
                this.historyModule.clearSidecarHistory(target);
            }
            this.sourcePath = target;
            this.sidecar = new this.historyModule.SidecarHistory(target);
        }
        else {
            this.sourcePath = target;
        }
        return report;
    }

    /** Projects this document into an exchange format. */
    export(options: ExportOptions): ExportReport {
        this.assertOpen();
        return service.exportDrawing(this.handle, options.format, options.target);
    }

    /** Lists every entity in this document. */
    listEntities(): readonly Entity[] {
        this.assertOpen();
        const result: EntityList = service.listEntities(this.handle);
        return result.entities;
    }

    /** Moves one entity within the paint order, back to front. */
    reorderEntity(entityId: EntityId, action: DrawOrderAction): DrawOrderResult {
        return this.edit((handle) => service.reorderEntity(handle, entityId, action), (result) => {
            this.state = result.drawing;
        });
    }

    /** Lists the paper-space layouts with their computed view. */
    listLayouts(): readonly Layout[] {
        this.assertOpen();
        return service.listLayouts(this.handle).layouts;
    }

    /** Creates one paper-space layout. */
    createLayout(request: CreateLayoutRequest): Layout {
        let layout: Layout | undefined;
        this.edit(
            (handle) => service.createLayout(handle, request),
            (result) => {
                this.state = result.drawing;
                layout = result.layout;
            },
        );
        return layout as Layout;
    }

    /** Updates one paper-space layout. */
    updateLayout(request: UpdateLayoutRequest): Layout {
        let layout: Layout | undefined;
        this.edit(
            (handle) => service.updateLayout(handle, request),
            (result) => {
                this.state = result.drawing;
                layout = result.layout;
            },
        );
        return layout as Layout;
    }

    /** Removes one paper-space layout. */
    deleteLayout(layoutId: string): LayoutDeletion {
        return this.edit((handle) => service.deleteLayout(handle, layoutId), (result) => {
            this.state = result.drawing;
        });
    }

    /** Adds a line and returns the created entity. */
    addLine(request: AddLineRequest): Entity {
        return this.adopt(this.edit((handle) => service.addLine(handle, request)));
    }

    /** Adds a point entity and returns the created entity. */
    addPoint(request: AddPointRequest): Entity {
        return this.adopt(this.edit((handle) => service.addPoint(handle, request)));
    }

    /** Adds a circle and returns the created entity. */
    addCircle(request: AddCircleRequest): Entity {
        return this.adopt(this.edit((handle) => service.addCircle(handle, request)));
    }

    /** Adds a circular arc and returns the created entity. */
    addArc(request: AddArcRequest): Entity {
        return this.adopt(this.edit((handle) => service.addArc(handle, request)));
    }

    /** Adds a full or partial ellipse and returns the created entity. */
    addEllipse(request: AddEllipseRequest): Entity {
        return this.adopt(this.edit((handle) => service.addEllipse(handle, request)));
    }

    /** Adds a polyline and returns the created entity. */
    addPolyline(request: AddPolylineRequest): Entity {
        return this.adopt(this.edit((handle) => service.addPolyline(handle, request)));
    }

    /** Adds single-line text and returns the created entity. */
    addText(request: AddTextRequest): Entity {
        return this.adopt(this.edit((handle) => service.addText(handle, request)));
    }

    /** Adds a linear dimension and returns the created entity. */
    addDimension(request: AddDimensionRequest): Entity {
        return this.adopt(this.edit((handle) => service.addDimension(handle, request)));
    }

    /** Adds a radial or diameter dimension and returns the created entity. */
    addRadialDimension(request: AddRadialDimensionRequest): Entity {
        return this.adopt(this.edit((handle) => service.addRadialDimension(handle, request)));
    }

    /** Adds an ordinate dimension and returns the created entity. */
    addOrdinateDimension(request: AddOrdinateDimensionRequest): Entity {
        return this.adopt(this.edit((handle) => service.addOrdinateDimension(handle, request)));
    }

    /** Adds a filled hatch bounded by one closed polygon. */
    addHatch(request: AddHatchRequest): Entity {
        return this.adopt(this.edit((handle) => service.addHatch(handle, request)));
    }

    /** Deletes one entity. */
    deleteEntity(entityId: string): EntityDeletion {
        return this.edit((handle) => service.deleteEntity(handle, entityId), (result) => {
            this.state = result.drawing;
        });
    }

    /** Restores the previous snapshot. Rejects with `drawing/nothing-to-undo` when empty. */
    undo(): HistoryResult {
        this.assertOpen();
        if (this.sidecar !== undefined) {
            return this.restore('undo');
        }
        const result = service.undo(this.handle);
        this.state = result.drawing;
        return result;
    }

    /** Reapplies the snapshot the last undo replaced. */
    redo(): HistoryResult {
        this.assertOpen();
        if (this.sidecar !== undefined) {
            return this.restore('redo');
        }
        const result = service.redo(this.handle);
        this.state = result.drawing;
        return result;
    }

    /** Adds one layer and returns its summary. */
    addLayer(request: AddLayerRequest): LayerSummary {
        let layer: LayerSummary | undefined;
        this.edit(
            (handle) => service.addLayer(handle, request),
            (result) => {
                this.state = result.drawing;
                layer = result.layer;
            },
        );
        return layer as LayerSummary;
    }

    /** Updates one layer. A rename also updates every entity reference. */
    updateLayer(request: UpdateLayerRequest): LayerSummary {
        let layer: LayerSummary | undefined;
        this.edit(
            (handle) => service.updateLayer(handle, request),
            (result) => {
                this.state = result.drawing;
                layer = result.layer;
            },
        );
        return layer as LayerSummary;
    }

    /** Moves every entity of one layer onto another and removes the empty source. */
    mergeLayer(request: MergeLayerRequest): LayerMerge {
        return this.edit((handle) => service.mergeLayer(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Removes one empty layer. */
    deleteLayer(layerId: string): LayerDeletion {
        return this.edit((handle) => service.deleteLayer(handle, layerId), (result) => {
            this.state = result.drawing;
        });
    }

    /** Lists the dimension styles of this document. */
    listDimStyles(): readonly DimensionStyle[] {
        this.assertOpen();
        return service.listDimStyles(this.handle).dimStyles;
    }

    /** Adds one dimension style and returns it. */
    addDimStyle(request: AddDimensionStyleRequest): DimensionStyle {
        let style: DimensionStyle | undefined;
        this.edit(
            (handle) => service.addDimStyle(handle, request),
            (result) => {
                this.state = result.drawing;
                style = result.style;
            },
        );
        return style as DimensionStyle;
    }

    /** Updates one dimension style and returns it. */
    updateDimStyle(request: UpdateDimensionStyleRequest): DimensionStyle {
        let style: DimensionStyle | undefined;
        this.edit(
            (handle) => service.updateDimStyle(handle, request),
            (result) => {
                this.state = result.drawing;
                style = result.style;
            },
        );
        return style as DimensionStyle;
    }

    /** Removes one dimension style. */
    deleteDimStyle(styleId: string): DimensionStyleDeletion {
        return this.edit((handle) => service.deleteDimStyle(handle, styleId), (result) => {
            this.state = result.drawing;
        });
    }

    /** Lists the text styles of this document. */
    listTextStyles(): readonly TextStyle[] {
        this.assertOpen();
        return service.listTextStyles(this.handle).textStyles;
    }

    /** Adds one text style and returns it. */
    addTextStyle(request: AddTextStyleRequest): TextStyle {
        let style: TextStyle | undefined;
        this.edit(
            (handle) => service.addTextStyle(handle, request),
            (result) => {
                this.state = result.drawing;
                style = result.style;
            },
        );
        return style as TextStyle;
    }

    /** Updates one text style and returns it. */
    updateTextStyle(request: UpdateTextStyleRequest): TextStyle {
        let style: TextStyle | undefined;
        this.edit(
            (handle) => service.updateTextStyle(handle, request),
            (result) => {
                this.state = result.drawing;
                style = result.style;
            },
        );
        return style as TextStyle;
    }

    /** Removes one text style. */
    deleteTextStyle(styleId: string): TextStyleDeletion {
        return this.edit((handle) => service.deleteTextStyle(handle, styleId), (result) => {
            this.state = result.drawing;
        });
    }

    /** Lists the line types of this document. */
    listLinetypes(): readonly Linetype[] {
        this.assertOpen();
        return service.listLinetypes(this.handle).linetypes;
    }

    /** Adds one line type and returns it. */
    addLinetype(request: AddLinetypeRequest): Linetype {
        let linetype: Linetype | undefined;
        this.edit(
            (handle) => service.addLinetype(handle, request),
            (result) => {
                this.state = result.drawing;
                linetype = result.linetype;
            },
        );
        return linetype as Linetype;
    }

    /** Updates one line type and returns it. A rename rewrites layer references. */
    updateLinetype(request: UpdateLinetypeRequest): Linetype {
        let linetype: Linetype | undefined;
        this.edit(
            (handle) => service.updateLinetype(handle, request),
            (result) => {
                this.state = result.drawing;
                linetype = result.linetype;
            },
        );
        return linetype as Linetype;
    }

    /** Removes one line type. */
    deleteLinetype(linetypeId: string): LinetypeDeletion {
        return this.edit((handle) => service.deleteLinetype(handle, linetypeId), (result) => {
            this.state = result.drawing;
        });
    }

    /** Lists the entity groups of this document. */
    listGroups(): readonly EntityGroup[] {
        this.assertOpen();
        return service.listGroups(this.handle).groups;
    }

    /** Creates one entity group from existing entities. */
    createGroup(request: CreateGroupRequest): EntityGroup {
        let group: EntityGroup | undefined;
        this.edit(
            (handle) => service.createGroup(handle, request),
            (result) => {
                this.state = result.drawing;
                group = result.group;
            },
        );
        return group as EntityGroup;
    }

    /** Removes one entity group. */
    deleteGroup(groupId: string): GroupDeletion {
        return this.edit((handle) => service.deleteGroup(handle, groupId), (result) => {
            this.state = result.drawing;
        });
    }

    /** Adds existing entities to one group. */
    addGroupMembers(request: GroupMembersRequest): EntityGroup {
        let group: EntityGroup | undefined;
        this.edit(
            (handle) => service.addGroupMembers(handle, request),
            (result) => {
                this.state = result.drawing;
                group = result.group;
            },
        );
        return group as EntityGroup;
    }

    /** Removes entities from one group. */
    removeGroupMembers(request: GroupMembersRequest): EntityGroup {
        let group: EntityGroup | undefined;
        this.edit(
            (handle) => service.removeGroupMembers(handle, request),
            (result) => {
                this.state = result.drawing;
                group = result.group;
            },
        );
        return group as EntityGroup;
    }

    /** Moves one entity by an offset or to an absolute anchor. */
    moveEntity(request: MoveEntityRequest): Entity {
        return this.adopt(this.edit((handle) => service.moveEntity(handle, request)));
    }

    /** Updates the content/height of text or the radius of a circle or arc. */
    updateEntity(request: UpdateEntityRequest): Entity {
        return this.adopt(this.edit((handle) => service.updateEntity(handle, request)));
    }

    /** Edits the vertices or closed flag of one polyline. */
    updatePolyline(request: UpdatePolylineRequest): Entity {
        return this.adopt(this.edit((handle) => service.updatePolyline(handle, request)));
    }

    /** Finds text entities whose content contains a pattern. */
    findText(request: FindTextRequest): TextMatchResult {
        this.assertOpen();
        return service.findText(this.handle, request);
    }

    /** Replaces a pattern in every matching text entity. */
    replaceText(request: ReplaceTextRequest): TextReplaceResult {
        return this.edit((handle) => service.replaceText(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Applies one affine transform to a selection and refreshes the summary. */
    transform(request: TransformRequest): TransformResult {
        return this.edit((handle) => service.transform(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Duplicates entities with a cumulative offset. */
    copy(request: CopyRequest): CopyResult {
        return this.edit((handle) => service.copy(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Duplicates entities into a rectangular or polar array. */
    array(request: ArrayRequest): ArrayResult {
        return this.edit((handle) => service.array(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Replaces one polyline with independent line entities. */
    explode(entityId: string): ExplodeResult {
        return this.edit((handle) => service.explode(handle, entityId), (result) => {
            this.state = result.drawing;
        });
    }

    /** Offsets a circle, arc, polyline or line into a new entity on the same layer. */
    offset(request: OffsetRequest): OffsetResult {
        return this.edit((handle) => service.offset(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Fillets two lines, trimming both, and returns the new arc. */
    fillet(first: EntityId, second: EntityId, radius: number): Entity {
        return this.adopt(this.edit((handle) => service.fillet(handle, first, second, radius)));
    }

    /** Chamfers two lines, trimming both, and returns the new bevel line. */
    chamfer(first: EntityId, second: EntityId, distanceA: number, distanceB: number): Entity {
        return this.adopt(this.edit((handle) => service.chamfer(handle, first, second, distanceA, distanceB)));
    }

    /** Measures bounds, path length and enclosed area. */
    measure(sheet?: string): MeasureResult {
        this.assertOpen();
        return service.measure(this.handle, sheet);
    }

    /** Measures the path length and enclosed area of one or more entities. */
    measureEntities(ids: readonly EntityId[]): EntityMeasureResult {
        this.assertOpen();
        return service.measureEntities(this.handle, ids);
    }

    /** Measures the distance between two points or the angle at three points. */
    measurePoints(points: readonly Point[]): PointMeasurement {
        this.assertOpen();
        return service.measurePoints(this.handle, points);
    }

    /** Converts points and entities between world and user coordinates. */
    ucs(request: UcsRequest = {}): UcsResult {
        this.assertOpen();
        if ((request.ids?.length ?? 0) === 0) {
            return service.ucs(this.handle, request);
        }
        return this.edit((handle) => service.ucs(handle, request), (result) => {
            if (result.drawing !== undefined) {
                this.state = result.drawing;
            }
        });
    }

    /** Summarises entity count, path length and enclosed area per layer. */
    stats(): DrawingStats {
        this.assertOpen();
        return service.stats(this.handle);
    }

    /** Selects entities with a rectangular window or crossing box. */
    select(request: SelectionRequest): SelectionResult {
        this.assertOpen();
        return service.select(this.handle, request);
    }

    /** Computes object snap candidates without changing the drawing. */
    snap(request: SnapRequest = {}): SnapResult {
        this.assertOpen();
        return service.snap(this.handle, request);
    }

    /** Converts every length between millimetres and inches. */
    convertUnits(request: ConvertUnitsRequest): UnitsConversion {
        this.assertOpen();
        if (this.state.units === request.to) {
            const unchanged = service.convertUnits(this.handle, request);
            this.state = unchanged.drawing;
            return unchanged;
        }
        return this.edit((handle) => service.convertUnits(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Releases the native handle. Idempotent. */
    close(): void {
        if (this.closed) {
            return;
        }
        this.closed = true;
        this.closeHandle();
    }

    /**
     * Runs one mutation. In a sidecar session it reads the current project
     * file first, writes the mutated document back on success and records the
     * replaced content for undo.
     */
    private edit<T>(work: (handle: number) => T, adopt: (value: T) => void = () => undefined): T {
        this.assertOpen();
        const sidecar = this.sidecar;
        const previous = sidecar === undefined ? undefined : sidecar.read();
        const result = work(this.handle);
        adopt(result);
        if (sidecar !== undefined && this.sourcePath !== undefined) {
            service.save(this.handle, this.sourcePath, true);
            sidecar.record(previous as string);
        }
        return result;
    }

    /** Swaps the file with a sidecar snapshot, then refreshes the handle. */
    private restore(direction: 'undo' | 'redo'): HistoryResult {
        const sidecar = this.sidecar as import('./history.js').SidecarHistory;
        const restored = sidecar.swap(direction);
        if (restored === undefined) {
            throw new OpenCADError(
                direction === 'undo' ? 'drawing/nothing-to-undo' : 'drawing/nothing-to-redo',
                direction === 'undo' ? 'the drawing has no earlier state to restore' : 'the drawing has no undone step to reapply',
            );
        }
        this.reopen();
        const counts = sidecar.counts();
        return {
            revision: this.state.revision,
            drawing: this.state,
            canUndo: counts.undo > 0,
            canRedo: counts.redo > 0,
        };
    }

    /** Closes the handle behind the restored file and opens the file again. */
    private reopen(): void {
        this.closeHandle();
        try {
            const opened = service.open(this.sourcePath as string);
            this.handleValue = opened.handle;
            this.state = opened.drawing;
        }
        catch (error) {
            this.closed = true;
            throw error;
        }
    }

    private closeHandle(): void {
        try {
            service.close(this.handleValue);
        } catch {
            // Releasing must not mask the caller's outcome; the handle is gone.
        }
    }

    private adopt(change: { readonly drawing: Drawing; readonly entity: Entity }): Entity {
        this.state = change.drawing;
        return change.entity;
    }

    private assertOpen(): void {
        if (this.closed) {
            throw new OpenCADError('drawing/not-found', `drawing handle ${this.handleValue} has been closed`);
        }
    }
}
