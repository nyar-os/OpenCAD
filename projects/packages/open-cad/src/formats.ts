/**
 * Drawing format registry.
 *
 * The registry classifies extensions the product must recognise and fixes what
 * each one is. Classification is **not** a support claim: what the installed
 * engine can read, write or project is reported by `OpenCAD.capabilities()`.
 * Today SVG, DXF, PDF, STL, OBJ and STEP are supported export targets, DXF
 * has a declared ASCII import subset and SVG has a declared element subset.
 * Classification of the other extensions
 * remains a description of the format, not a promise.
 */

/** Grouping used for capability reporting and user-facing wording. */
export type FormatFamily = 'native' | 'exchange' | 'mesh' | 'vector' | 'document' | 'raster';

/** Container encoding behind an extension. */
export type FormatMarkup = 'binary' | 'text' | 'json' | 'xml' | 'fixed' | 'raster';

/** How an extension is expected to enter the workspace. `none` never opens as a drawing. */
export type FormatOpenKind = 'drawing' | 'reference' | 'none';

/** Every extension the product must classify. */
export const drawingFormats = [
    'dwg',
    'dxf',
    'step',
    'stp',
    'iges',
    'igs',
    'stl',
    'obj',
    'svg',
    'pdf',
    'png',
    'jpg',
] as const;

/** Union of {@link drawingFormats}. */
export type DrawingFormat = (typeof drawingFormats)[number];

/** What a format is, independent of whether it is implemented. */
export interface DrawingFormatInfo {
    readonly format: DrawingFormat;
    readonly family: FormatFamily;
    readonly markup: FormatMarkup;
    readonly opensAs: FormatOpenKind;
    /** Whether the format is an exchange target `export()` may write. */
    readonly exchangeTarget: boolean;
}

type FormatRegistryEntry = Omit<DrawingFormatInfo, 'format'>;

/** Extension to classification. Values describe the format, never a capability. */
export const formatRegistry: Readonly<Record<DrawingFormat, FormatRegistryEntry>> = Object.freeze({
    dwg: { family: 'native', markup: 'binary', opensAs: 'drawing', exchangeTarget: false },
    dxf: { family: 'exchange', markup: 'text', opensAs: 'drawing', exchangeTarget: true },
    step: { family: 'exchange', markup: 'text', opensAs: 'drawing', exchangeTarget: true },
    stp: { family: 'exchange', markup: 'text', opensAs: 'drawing', exchangeTarget: true },
    iges: { family: 'exchange', markup: 'text', opensAs: 'drawing', exchangeTarget: true },
    igs: { family: 'exchange', markup: 'text', opensAs: 'drawing', exchangeTarget: true },
    stl: { family: 'mesh', markup: 'binary', opensAs: 'reference', exchangeTarget: true },
    obj: { family: 'mesh', markup: 'text', opensAs: 'reference', exchangeTarget: true },
    svg: { family: 'vector', markup: 'xml', opensAs: 'drawing', exchangeTarget: true },
    pdf: { family: 'document', markup: 'fixed', opensAs: 'reference', exchangeTarget: true },
    png: { family: 'raster', markup: 'raster', opensAs: 'reference', exchangeTarget: false },
    jpg: { family: 'raster', markup: 'raster', opensAs: 'reference', exchangeTarget: false },
});

/** Returns the full classification of one format. */
export function formatInfo(format: DrawingFormat): DrawingFormatInfo {
    return { format, ...formatRegistry[format] };
}

/** Extracts a classified extension from a path, ignoring case. */
export function formatFromPath(path: string): DrawingFormat | undefined {
    const extension = path.slice(path.lastIndexOf('.') + 1).toLowerCase();
    return (drawingFormats as readonly string[]).includes(extension) ? (extension as DrawingFormat) : undefined;
}

/** Whether `export()` may target this format. */
export function isExchangeTarget(format: DrawingFormat): boolean {
    return formatRegistry[format].exchangeTarget;
}
