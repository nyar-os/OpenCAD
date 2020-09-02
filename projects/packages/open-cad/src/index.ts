/**
 * Drawing surface of `@wae/open-cad`.
 *
 * The package wraps the native document core: create/open/save round trips,
 * entity editing, undo/redo, layer management, inspection, the declared ASCII
 * DXF import subset and SVG/DXF/PDF export work today. The interactive
 * viewport, STEP/STL output and a full DXF reader are not implemented and are
 * reported as such by `capabilities()`.
 */

export { DrawingDocument } from './document.js';
export { OpenCAD } from './open-cad.js';
export { closeGui, defaultGuiContentDir, GuiError, isGuiOpen, openGui, pumpEvents, runGui } from './gui.js';
export type { GuiCloseResult, GuiOpenResult, GuiStatus, OpenGuiOptions } from './gui.js';
export { NotImplementedError, OpenCADError, errorCodes, isOpenCADError } from './errors.js';
export type { OpenCADErrorCode, OpenCADErrorDetails, OpenCADErrorOptions } from './errors.js';
export { drawingFormats, formatFromPath, formatInfo, formatRegistry, isExchangeTarget } from './formats.js';
export type { DrawingFormat, DrawingFormatInfo, FormatFamily, FormatMarkup, FormatOpenKind } from './formats.js';
export { capabilities } from './service.js';
export type * from './types.js';
