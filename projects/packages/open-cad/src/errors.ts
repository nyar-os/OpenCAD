/**
 * Stable error surface of `@wae/open-cad`.
 *
 * Codes are contract, not prose: callers compare them, localize around them and
 * never parse the message. A failure therefore keeps its code even when the
 * text is translated.
 */

/** Every stable code the drawing surface may report. */
export const errorCodes = [
    'session/disposed',
    'session/argument-invalid',
    'session/budget-exceeded',
    'drawing/not-found',
    'drawing/revision-conflict',
    'drawing/layer-locked',
    'drawing/layer-frozen',
    'drawing/layer-exists',
    'drawing/style-exists',
    'drawing/group-exists',
    'drawing/linetype-exists',
    'drawing/style-last',
    'drawing/linetype-last',
    'drawing/linetype-in-use',
    'drawing/nothing-to-undo',
    'drawing/nothing-to-redo',
    'drawing/overwrite-refused',
    'drawing/io-error',
    'object/not-found',
    'object/unsupported',
    'offset/invalid-distance',
    'offset/invalid-side',
    'offset/miter-limit',
    'offset/not-found',
    'offset/unsupported-entity',
    'trim/degenerate',
    'trim/no-intersection',
    'extend/no-intersection',
    'break/not-on-entity',
    'join/not-adjacent',
    'align/insufficient-entities',
    'distribute/insufficient-entities',
    'hatch/invalid-angle',
    'hatch/invalid-spacing',
    'hatch/unsupported-entity',
    'hatch/solid-pattern',
    'hatch/degenerate-boundary',
    'hatch/spacing-too-fine',
    'section/empty',
    'titleblock/invalid-sheet',
    'titleblock/content-overflow',
    'hole-table/empty',
    'detail-view/empty',
    'dimension/unsupported-entity',
    'polyline/invalid-iterations',
    'polyline/invalid-ratio',
    'polyline/degenerate',
    'polyline/unsupported-entity',
    'polyline/vertex-limit',
    'object/degenerate',
    'format/unsupported',
    'format/unsupported-version',
    'format/invalid',
    'capability/not-implemented',
] as const;

/** Union of {@link errorCodes}. */
export type OpenCADErrorCode = (typeof errorCodes)[number];

/** Optional locators a failure can carry so callers can point at the object. */
export interface OpenCADErrorDetails {
    readonly revision?: number;
    readonly layerId?: string;
    readonly entityId?: string;
    /** Drawing format involved in the failure, without the leading dot. */
    readonly format?: string;
    /** Capability involved in the failure, e.g. `drawing.export`. */
    readonly feature?: string;
}

/** Options accepted by {@link OpenCADError}. */
export interface OpenCADErrorOptions {
    readonly recoverable?: boolean;
    readonly details?: OpenCADErrorDetails;
    readonly cause?: unknown;
}

/** Every failure thrown or rejected by this package. */
export class OpenCADError extends Error {
    /** Stable, comparable code. */
    readonly code: OpenCADErrorCode;
    /** Whether retrying with different input or state can succeed. */
    readonly recoverable: boolean;
    /** Optional locators (`revision`, `layerId`, `entityId`, `format`, `feature`). */
    readonly details: OpenCADErrorDetails;

    constructor(code: OpenCADErrorCode, message: string, options: OpenCADErrorOptions = {}) {
        super(message, options.cause === undefined ? undefined : { cause: options.cause });
        this.name = 'OpenCADError';
        this.code = code;
        this.recoverable = options.recoverable ?? false;
        this.details = Object.freeze({ ...options.details });
    }
}

/**
 * Reported when the optional native addon is absent or a declared route is not
 * implemented by the installed build.
 *
 * The document core, inspection, undo/redo, layer editing, the ASCII DXF reader
 * and SVG/DXF/PDF export do not use this code. It remains for STEP/STL export
 * and for an uninstalled addon.
 */
export class NotImplementedError extends OpenCADError {
    constructor(feature: string, message = `\`${feature}\` is not implemented yet`) {
        super('capability/not-implemented', message, { recoverable: false, details: { feature } });
        this.name = 'NotImplementedError';
    }
}

/** Narrows an unknown thrown value to {@link OpenCADError}. */
export function isOpenCADError(value: unknown): value is OpenCADError {
    return value instanceof OpenCADError;
}
