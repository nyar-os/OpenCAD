/**
 * Native addon discovery.
 *
 * The addon only maps values: the drawing document, persistence, inspection,
 * undo/redo history, layer and entity editing, the ASCII DXF reader and
 * exchange live in the shared Rust model crate. An absent addon is a
 * declarative state, so calls report `capability/not-implemented` instead of
 * inventing a drawing.
 */

import { createRequire } from 'node:module';

const requireNative = createRequire(import.meta.url);

/** Orthogonal exports of the built addon. Every drawing call returns a JSON string. */
export interface NativeBinding {
    version(): string;
    capabilities(): string;
    drawing_create(requestJson: string): string;
    drawing_open(path: string): string;
    drawing_save(handle: number, path: string, overwrite: boolean): string;
    drawing_inspect(handle: number, checksJson?: string): string;
    drawing_export(handle: number, format: string, path: string): string;
    drawing_close(handle: number): boolean;
    drawing_add_line(handle: number, requestJson: string): string;
    drawing_add_circle(handle: number, requestJson: string): string;
    drawing_add_arc(handle: number, requestJson: string): string;
    drawing_add_ellipse(handle: number, requestJson: string): string;
    drawing_add_polyline(handle: number, requestJson: string): string;
    drawing_add_text(handle: number, requestJson: string): string;
    drawing_add_dimension(handle: number, requestJson: string): string;
    drawing_delete_entity(handle: number, entityId: string): string;
    drawing_list_entities(handle: number): string;
    drawing_undo(handle: number): string;
    drawing_redo(handle: number): string;
    drawing_add_layer(handle: number, requestJson: string): string;
    drawing_update_layer(handle: number, requestJson: string): string;
    drawing_move_entity(handle: number, requestJson: string): string;
    drawing_update_entity(handle: number, requestJson: string): string;
    drawing_transform(handle: number, requestJson: string): string;
    drawing_copy(handle: number, requestJson: string): string;
    drawing_measure(handle: number, sheet?: string): string;
    drawing_import_dxf(requestJson: string): string;
    openGui(options?: { title?: string; contentDir?: string; width?: number; height?: number }): string;
    pumpEvents(): string;
    closeGui(): string;
    isGuiOpen(): boolean;
}

let cached: NativeBinding | null | undefined;

/** Loads the addon once, or returns `undefined` when it is not installed. */
export function nativeBinding(): NativeBinding | undefined {
    if (cached === undefined) {
        cached = load();
    }
    return cached ?? undefined;
}

function load(): NativeBinding | null {
    for (const candidate of candidates()) {
        try {
            return requireNative(candidate) as NativeBinding;
        } catch (error) {
            if (candidate === explicitPath()) {
                throw error;
            }
            if (!isModuleNotFound(error, candidate)) {
                throw error;
            }
        }
    }
    return null;
}

function candidates(): string[] {
    const explicit = explicitPath();
    if (explicit) {
        return [explicit];
    }
    return [`@wae/open-cad-${process.platform}-${process.arch}`];
}

function explicitPath(): string | undefined {
    const value = process.env.OPEN_CAD_NATIVE;
    return value !== undefined && value.length > 0 ? value : undefined;
}

function isModuleNotFound(error: unknown, candidate: string): boolean {
    if (typeof error !== 'object' || error === null) {
        return false;
    }
    const code = (error as { code?: unknown }).code;
    // Node prints the resolved path, which uses the platform separator on
    // Windows. Normalize it so a linked platform package without a built
    // addon is still treated as "not installed" instead of a fatal error.
    const message = (error as Error).message.replace(/\\/g, '/');
    return code === 'MODULE_NOT_FOUND' && message.includes(candidate);
}
