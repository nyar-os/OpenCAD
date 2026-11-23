/**
 * `new OpenCAD(options?)` drawing session.
 *
 * The constructor is synchronous and opens nothing: no network, no disk scan,
 * no window, no file. Native initialisation and imports happen inside the async
 * methods, so a failure is always awaited and always carries a stable code.
 *
 * `create()` and `open()` return a {@link DrawingDocument}; the document owns
 * the native handle and exposes inspection, save, export and the editing calls.
 */

import { DrawingDocument } from './document.js';
import { OpenCADError } from './errors.js';
import * as service from './service.js';
import type {
    DrawingCreateOptions,
    DrawingDiffRequest,
    DrawingDiffResult,
    DrawingInput,
    HistoryMode,
    ImportDxfRequest,
    InspectOptions,
    InspectReport,
    OpenCADCapabilities,
    OpenCADOptions,
} from './types.js';

type HistoryModule = typeof import('./history.js');

/** Drawing session. One instance owns the documents it opens. */
export class OpenCAD {
    /** BCP-47 tag used for diagnostics and default copy. */
    readonly locale: string;
    /** Undo/redo backing store of every document this session opens. */
    readonly history: HistoryMode;
    private readonly documents = new Set<DrawingDocument>();
    private historyModule: HistoryModule | undefined;
    private closed = false;

    constructor(options: OpenCADOptions = {}) {
        this.locale = options.locale ?? 'en';
        this.history = options.history ?? 'memory';
    }

    /** Whether {@link dispose} has run. */
    get disposed(): boolean {
        return this.closed;
    }

    /**
     * Format, check, renderer and export disclosure for the installed engine.
     * Read this instead of treating a method name as a capability.
     */
    async capabilities(): Promise<OpenCADCapabilities> {
        this.assertActive();
        return service.capabilities();
    }

    /** Creates an empty drawing and returns its document. */
    async create(options?: DrawingCreateOptions): Promise<DrawingDocument> {
        this.assertActive();
        if (options?.title !== undefined && typeof options.title !== 'string') {
            throw new OpenCADError('session/argument-invalid', '`title` must be a string', {
                details: { feature: 'create.title' },
            });
        }
        if (options?.units !== undefined && options.units !== 'mm' && options.units !== 'inch') {
            throw new OpenCADError('session/argument-invalid', '`units` must be `mm` or `inch`', {
                details: { feature: 'create.units' },
            });
        }
        const historyModule = await this.loadHistory();
        const created = service.create({ title: options?.title, units: options?.units, template: options?.template });
        const document = new DrawingDocument(created.handle, created.drawing, { history: this.history, historyModule });
        this.documents.add(document);
        return document;
    }

    /** Opens a project file and returns its document. */
    async open(input: DrawingInput): Promise<DrawingDocument> {
        this.assertActive();
        assertDrawingInput(input);
        const historyModule = await this.loadHistory();
        const opened = service.open(input.path);
        const document = new DrawingDocument(opened.handle, opened.drawing, {
            history: this.history,
            sourcePath: input.path,
            historyModule,
        });
        this.documents.add(document);
        return document;
    }

    /**
     * Imports the declared ASCII DXF subset into a new document.
     *
     * The new handle starts with an empty history. The returned document
     * exposes the imported counts and the losses through its `imported` and
     * `losses` getters.
     */
    async importDxf(input: ImportDxfRequest): Promise<DrawingDocument> {
        this.assertActive();
        const candidate = input as unknown as Record<string, unknown> | null;
        if (typeof candidate !== 'object' || candidate === null) {
            throw new OpenCADError('session/argument-invalid', 'ImportDxfRequest must contain `source`');
        }
        if (typeof candidate.source !== 'string' || candidate.source.length === 0) {
            throw new OpenCADError('session/argument-invalid', '`source` must be a non-empty string');
        }
        const historyModule = await this.loadHistory();
        const imported = service.importDxf({ source: candidate.source, title: typeof candidate.title === 'string' ? candidate.title : undefined });
        const document = new DrawingDocument(imported.handle, imported.drawing, {
            history: this.history,
            historyModule,
            imported: imported.imported,
            losses: imported.losses,
        });
        this.documents.add(document);
        return document;
    }

    /** Inspects an input and releases the transient handle before returning. */
    async inspect(input: DrawingInput, options?: InspectOptions): Promise<InspectReport> {
        this.assertActive();
        assertDrawingInput(input);
        const opened = service.open(input.path);
        try {
            return service.inspect(opened.handle, options?.checks);
        } finally {
            this.release(opened.handle);
        }
    }

    /**
     * Compares two `.ocad` projects at entity level without opening a handle.
     *
     * Entities and layers are matched by their stable id, so the report lists
     * what only the right side carries, what only the left side carries and
     * what both carry with different fields. Metadata covers the title, the
     * unit and the declared properties. Revisions, styles, line types, groups,
     * layouts and draw order are not compared. A missing or malformed file
     * reports `drawing/not-found`, `drawing/io-error` or `format/invalid`.
     */
    async diff(request: DrawingDiffRequest): Promise<DrawingDiffResult> {
        this.assertActive();
        const candidate = request as unknown as Record<string, unknown> | null;
        if (typeof candidate !== 'object' || candidate === null) {
            throw new OpenCADError('session/argument-invalid', 'DrawingDiffRequest must contain `left` and `right`');
        }
        for (const side of ['left', 'right'] as const) {
            const value = candidate[side];
            if (typeof value !== 'string' || value.length === 0) {
                throw new OpenCADError('session/argument-invalid', `\`${side}\` must be a non-empty path`, {
                    details: { feature: `diff.${side}` },
                });
            }
        }
        return service.diff({ left: candidate.left as string, right: candidate.right as string });
    }

    /** Releases every document the session still owns. Idempotent. */
    async dispose(): Promise<void> {
        if (this.closed) {
            return;
        }
        this.closed = true;
        for (const document of [...this.documents]) {
            document.close();
        }
        this.documents.clear();
    }

    /** Loads the sidecar helpers once, and only for `history: 'sidecar'`. */
    private async loadHistory(): Promise<HistoryModule | undefined> {
        if (this.history !== 'sidecar') {
            return undefined;
        }
        if (this.historyModule === undefined) {
            this.historyModule = await import('./history.js');
        }
        return this.historyModule;
    }

    /** Closes one transient handle, keeping a failed release from masking the caller. */
    private release(handle: number): void {
        try {
            service.close(handle);
        } catch {
            // The addon may be absent or the engine may already have dropped it.
        }
    }

    /** @internal */
    assertActive(): void {
        if (this.closed) {
            throw new OpenCADError('session/disposed', 'the OpenCAD session has been disposed; create a new instance');
        }
    }
}

function assertDrawingInput(input: DrawingInput): void {
    const candidate = input as unknown as Record<string, unknown>;
    if (typeof candidate !== 'object' || candidate === null || !('path' in candidate)) {
        throw new OpenCADError('session/argument-invalid', 'DrawingInput must contain `path`');
    }
    const value = candidate.path;
    if (typeof value !== 'string' || value.length === 0) {
        throw new OpenCADError('session/argument-invalid', '`path` must be a non-empty string');
    }
}
