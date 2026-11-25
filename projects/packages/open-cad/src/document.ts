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
    AddAngularDimensionRequest,
    AddPointRequest,
    AddPolygonRequest,
    AddPolylineRequest,
    AddRadialDimensionRequest,
    AddRectangleRequest,
    AddSplineRequest,
    AddTextRequest,
    AddTextStyleRequest,
    AlignRequest,
    AlignResult,
    ArrayRequest,
    ArrayResult,
    AssignEntityLayerRequest,
    BreakResult,
    CopyRequest,
    CopyResult,
    ConvertUnitsRequest,
    CreateGroupRequest,
    CreateLayoutRequest,
    DimensionStyle,
    DimensionStyleDeletion,
    DistributeRequest,
    DistributeResult,
    Drawing,
    DrawingProperties,
    DrawOrderAction,
    DrawOrderResult,
    Entity,
    EntityDeletion,
    EntityGroup,
    EntityId,
    EntityLayerChange,
    EntityList,
    EntityMeasureResult,
    DrawingStats,
    ExplodeResult,
    ExportOptions,
    ExportReport,
    ExtendResult,
    ExtendSide,
    FindTextRequest,
    GroupDeletion,
    GroupMembersRequest,
    HatchPatternRequest,
    HatchPatternResult,
    HistoryMode,
    HistoryResult,
    ImportLoss,
    InspectOptions,
    InspectReport,
    JoinResult,
    LayerDeletion,
    LayerMerge,
    LayerSummary,
    Layout,
    LayoutDeletion,
    Linetype,
    LinetypeDeletion,
    OffsetRequest,
    OffsetResult,
    OffsetBySideRequest,
    OffsetBySideResult,
    MeasureResult,
    OverkillRequest,
    OverkillResult,
    Point,
    PointMeasurement,
    PolylineChamferRequest,
    QuantityTableRequest,
    QuantityTableResult,
    LeaderNoteRequest,
    LeaderNoteResult,
    ExplodedAssemblyRequest,
    ExplodedAssemblyResult,
    HoleTableRequest,
    HoleTableResult,
    IsometricViewRequest,
    IsometricViewResult,
    MultiViewSheetRequest,
    MultiViewSheetResult,
    SectionViewRequest,
    SectionViewResult,
    MergeLayerRequest,
    MoveEntityRequest,
    ReplaceTextRequest,
    SaveOptions,
    SaveReport,
    SelectionRequest,
    SelectionResult,
    SmoothPolylineRequest,
    SnapRequest,
    SnapResult,
    StatsGroupBy,
    StretchRequest,
    StretchResult,
    TextStyle,
    TextStyleDeletion,
    TextMatchResult,
    TextReplaceResult,
    TitleBlockRequest,
    TitleBlockResult,
    TransformRequest,
    TransformResult,
    TrimResult,
    UcsRequest,
    UcsResult,
    UnitsConversion,
    UpdateArcRequest,
    UpdateDimensionRequest,
    UpdateDimensionStyleRequest,
    UpdateEllipseRequest,
    UpdateEntityRequest,
    UpdateLayerRequest,
    UpdateLayoutRequest,
    UpdateLinetypeRequest,
    UpdatePolylineRequest,
    UpdatePropertiesRequest,
    UpdateSplineRequest,
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
    /** Import statistics when the document came from `importDxf()` or `importSvg()`. */
    readonly imported?: { readonly entities: number; readonly layers: number };
    /** Import losses reported by `importDxf()` or `importSvg()`. */
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

    /** Import statistics when this document came from `importDxf()` or `importSvg()`. */
    get imported(): { readonly entities: number; readonly layers: number } | undefined {
        return this.importedCounts;
    }

    /** Import losses when this document came from `importDxf()` or `importSvg()`. */
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

    /** Adds a cubic spline through its fit points and returns the created entity. */
    addSpline(request: AddSplineRequest): Entity {
        return this.adopt(this.edit((handle) => service.addSpline(handle, request)));
    }

    /** Adds an axis-aligned rectangle as one closed polyline. */
    addRectangle(request: AddRectangleRequest): Entity {
        return this.adopt(this.edit((handle) => service.addRectangle(handle, request)));
    }

    /** Adds a regular polygon as one closed polyline. */
    addPolygon(request: AddPolygonRequest): Entity {
        return this.adopt(this.edit((handle) => service.addPolygon(handle, request)));
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

    /** Adds an angular dimension swept counter-clockwise from `first` to `second`. */
    addAngularDimension(request: AddAngularDimensionRequest): Entity {
        return this.adopt(this.edit((handle) => service.addAngularDimension(handle, request)));
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

    /** Updates the radius or sweep angles of one arc. */
    updateArc(request: UpdateArcRequest): Entity {
        return this.adopt(this.edit((handle) => service.updateArc(handle, request)));
    }

    /** Edits the fit points or closed flag of one spline. */
    updateSpline(request: UpdateSplineRequest): Entity {
        return this.adopt(this.edit((handle) => service.updateSpline(handle, request)));
    }

    /**
     * Updates the radii, rotation or sweep parameters of one ellipse. Only the
     * supplied fields change; `rx` and `ry` must stay positive.
     */
    updateEllipse(request: UpdateEllipseRequest): Entity {
        return this.adopt(this.edit((handle) => service.updateEllipse(handle, request)));
    }

    /** Updates the measured points, offset or style of one linear dimension. */
    updateDimension(request: UpdateDimensionRequest): Entity {
        return this.adopt(this.edit((handle) => service.updateDimension(handle, request)));
    }

    /** Moves one entity onto another layer without changing its geometry. */
    assignEntityLayer(request: AssignEntityLayerRequest): EntityLayerChange {
        return this.edit((handle) => service.assignEntityLayer(handle, request), (result) => {
            this.state = result.drawing;
        });
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

    /**
     * Offsets a circle, arc, polyline or line by an explicit side. `side` is
     * `left`/`right` for a line or polyline and `outside`/`inside` for a circle
     * or arc; `distance` must be positive.
     */
    offsetBySide(request: OffsetBySideRequest): OffsetBySideResult {
        return this.edit((handle) => service.offsetBySide(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Trims a line at a line, circle or arc cutter, keeping the longer side.
     * Tangency, collinearity, a zero-length input and a cut on a target
     * endpoint report `trim/degenerate`; a cutter without a crossing reports
     * `trim/no-intersection`.
     */
    trim(id: EntityId, cutter: EntityId): TrimResult {
        return this.edit((handle) => service.trim(handle, id, cutter), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Extends a line past one endpoint until it meets a line, circle or arc.
     * `side` defaults to `end`; only crossings ahead of that endpoint count, so
     * the line grows to the nearest one. A boundary the ray never reaches
     * reports `extend/no-intersection`.
     */
    extend(id: EntityId, boundary: EntityId, side: ExtendSide = 'end'): ExtendResult {
        return this.edit((handle) => service.extend(handle, id, boundary, side), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Splits a line or arc at a point strictly inside it. A line point is
     * projected onto the segment, so both pieces stay collinear and their
     * lengths add up to the original. An arc point must lie on the arc away from
     * both endpoints and splits it at that angle into two arcs sharing the
     * centre and radius. Every piece stays on the source layer; a point on an
     * endpoint or off the entity reports `break/not-on-entity`, and a circle or
     * another kind reports `object/unsupported`.
     */
    breakEntity(id: EntityId, at: Point): BreakResult {
        return this.edit((handle) => service.breakEntity(handle, id, at), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Merges two collinear lines that share an endpoint into one line. Both must
     * be lines on the same layer, one endpoint of each must coincide, and the
     * lines must continue in opposite directions from that point. The two
     * sources are removed and replaced by one line from `a`'s free endpoint to
     * `b`'s free endpoint with a fresh id; endpoints that do not meet, lines that
     * are not collinear, or a folded-back overlap report `join/not-adjacent`.
     */
    joinEntities(a: EntityId, b: EntityId): JoinResult {
        return this.edit((handle) => service.joinEntities(handle, a, b), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Aligns a selection of entities to one bounding-box edge or centreline.
     * `left`, `right`, `bottom` and `top` pull the extreme edges of every box
     * onto the smallest or largest edge of the selection; `center` and `middle`
     * pull the box centres onto the midpoint of the selection's span. Entities
     * move by translation only; fewer than two distinct ids report
     * `align/insufficient-entities`.
     */
    align(request: AlignRequest): AlignResult {
        return this.edit((handle) => service.align(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Spreads a selection of entities with equal gaps between their bounding
     * boxes along `axis`. Entities are ordered by box centre; the first and
     * last stay put and every interior entity is translated so consecutive gaps
     * match. Fewer than three distinct ids report
     * `distribute/insufficient-entities`.
     */
    distribute(request: DistributeRequest): DistributeResult {
        return this.edit((handle) => service.distribute(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Stretches the vertices and endpoints inside a crossing window by `dx`/
     * `dy`, following the same crossing semantics as {@link select}. Line
     * endpoints, polyline, spline and hatch points, points and the centre of a
     * circle, arc or ellipse move when they lie inside the window (boundary
     * included); text and dimension entities are skipped and counted in
     * `skipped`, while a supported entity that crosses the window without a
     * vertex inside stays unchanged. Ids, layers and draw order stay put, and
     * the call is one revision and one history step.
     */
    stretch(request: StretchRequest): StretchResult {
        return this.edit((handle) => service.stretch(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Removes exact duplicate entities on one layer within `tolerance` and,
     * when `mergeOverlapping` is set, merges collinear overlapping or touching
     * lines into union lines. Text, dimensions and hatches are never compared
     * and are reported in `skipped`. A cleanup with changes is one history
     * step; a clean drawing is left untouched.
     */
    overkill(request: OverkillRequest = {}): OverkillResult {
        return this.edit((handle) => service.overkill(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Fills one hatch with pattern line segments clipped to its boundary.
     * The pattern lands on the hatch layer as real line entities, so every
     * exporter carries it. `angle` defaults to 0, `spacing` to 5 and `cross`
     * to false.
     */
    hatchPattern(request: HatchPatternRequest): HatchPatternResult {
        return this.edit((handle) => service.hatchPattern(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Adds a sheet frame and a title block filled from the drawing properties.
     * Defaults to an A3 landscape sheet, a 10-unit margin and a `titleblock`
     * layer; every field can be overridden.
     */
    titleBlock(request: TitleBlockRequest = {}): TitleBlockResult {
        return this.edit((handle) => service.titleBlock(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Rounds the corners of one polyline with Chaikin corner cutting. An open
     * chain keeps its endpoints, a closed chain keeps its closed flag, and the
     * entity keeps its id and layer. `iterations` defaults to 2 and `ratio` to
     * 0.25. A non-polyline reports `polyline/unsupported-entity` and a chain
     * with fewer than three vertices reports `polyline/degenerate`.
     */
    smoothPolyline(request: SmoothPolylineRequest): Entity {
        return this.adopt(this.edit((handle) => service.smoothPolyline(handle, request)));
    }

    /**
     * Bevels every corner of one polyline by trimming both adjacent segments.
     * An open chain keeps its endpoints, a closed chain bevels every vertex
     * including the first, and the entity keeps its id, layer and closure.
     * `distance` must be positive and finite and every segment must be longer
     * than twice it. A non-polyline reports `polyline/unsupported-entity`, a
     * chain with fewer than three vertices `polyline/degenerate` and a short
     * segment `session/argument-invalid`.
     */
    polylineChamfer(request: PolylineChamferRequest): Entity {
        return this.adopt(this.edit((handle) => service.polylineChamfer(handle, request)));
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

    /**
     * Summarises entity count, path length and enclosed area.
     *
     * `groupBy` defaults to `layer`, which keeps the layer order and includes
     * empty layers. `type`, `color` and `linetype` list only keys that occur,
     * sorted by key.
     */
    stats(groupBy: StatsGroupBy = 'layer'): DrawingStats {
        this.assertOpen();
        return service.stats(this.handle, groupBy);
    }

    /**
     * Draws a framed quantity take-off table of the drawing aggregates.
     *
     * One row per group of {@link stats}, then a total row, as real line and
     * text entities on a `quantity` layer by default. The `origin` option sets
     * the bottom-left corner; `textHeight` defaults to 3.5 and an empty `title`
     * omits the caption row.
     */
    quantityTable(request: QuantityTableRequest = {}): QuantityTableResult {
        return this.edit((handle) => service.quantityTable(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Projects a declarative part model onto a multiview sheet.
     *
     * First- or third-angle front, top and left views plus an isometric view,
     * with labels, hidden outlines from voids and the ISO projection symbol, on
     * a `views` layer by default. The `views` field reports the placed view
     * boxes; `origin` sets the lower-left corner of the group.
     */
    multiviewSheet(request: MultiViewSheetRequest): MultiViewSheetResult {
        return this.edit((handle) => service.multiviewSheet(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Cuts a declarative part model by a plane and draws the section view.
     *
     * The cut outline, 45-degree hatch lines, the section label and the
     * cutting-plane indicator, on a `views` layer by default. `axis` and `at`
     * place the cutting plane; `hatchSpacing` too fine reports
     * `hatch/spacing-too-fine`.
     */
    sectionView(request: SectionViewRequest): SectionViewResult {
        return this.edit((handle) => service.sectionView(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /**
     * Draws a polyline leader with an arrowhead and a note.
     *
     * `points` chain from the referenced object to the note; the first point
     * carries an arrowhead unless `arrow` is false. The note lands on a
     * `notes` layer by default.
     */
    leaderNote(request: LeaderNoteRequest): LeaderNoteResult {
        return this.edit((handle) => service.leaderNote(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Draws a standalone isometric view of a declarative part model with an axis indicator. */
    isometricView(request: IsometricViewRequest): IsometricViewResult {
        return this.edit((handle) => service.isometricView(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Draws an exploded assembly view of declarative components with numbered balloons. */
    explodedAssembly(request: ExplodedAssemblyRequest): ExplodedAssemblyResult {
        return this.edit((handle) => service.explodedAssembly(handle, request), (result) => {
            this.state = result.drawing;
        });
    }

    /** Tabulates the cylindrical voids of a declarative part model as a framed hole table. */
    holeTable(request: HoleTableRequest): HoleTableResult {
        return this.edit((handle) => service.holeTable(handle, request), (result) => {
            this.state = result.drawing;
        });
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
