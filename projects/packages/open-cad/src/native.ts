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
    drawing_add_point(handle: number, requestJson: string): string;
    drawing_add_circle(handle: number, requestJson: string): string;
    drawing_add_arc(handle: number, requestJson: string): string;
    drawing_add_ellipse(handle: number, requestJson: string): string;
    drawing_add_polyline(handle: number, requestJson: string): string;
    drawing_add_spline(handle: number, requestJson: string): string;
    drawing_add_rectangle(handle: number, requestJson: string): string;
    drawing_add_polygon(handle: number, requestJson: string): string;
    drawing_add_text(handle: number, requestJson: string): string;
    drawing_add_dimension(handle: number, requestJson: string): string;
    drawing_add_radial_dimension(handle: number, requestJson: string): string;
    drawing_add_ordinate_dimension(handle: number, requestJson: string): string;
    drawing_add_angular_dimension(handle: number, requestJson: string): string;
    drawing_add_hatch(handle: number, requestJson: string): string;
    drawing_delete_entity(handle: number, entityId: string): string;
    drawing_list_entities(handle: number): string;
    drawing_reorder_entity(handle: number, entityId: string, action: string): string;
    drawing_list_layouts(handle: number): string;
    drawing_create_layout(handle: number, requestJson: string): string;
    drawing_update_layout(handle: number, layoutId: string, requestJson: string): string;
    drawing_delete_layout(handle: number, layoutId: string): string;
    drawing_undo(handle: number): string;
    drawing_redo(handle: number): string;
    drawing_properties(handle: number): string;
    drawing_update_properties(handle: number, requestJson: string): string;
    drawing_add_layer(handle: number, requestJson: string): string;
    drawing_update_layer(handle: number, requestJson: string): string;
    drawing_delete_layer(handle: number, layerId: string): string;
    drawing_list_dim_styles(handle: number): string;
    drawing_add_dim_style(handle: number, requestJson: string): string;
    drawing_update_dim_style(handle: number, requestJson: string): string;
    drawing_delete_dim_style(handle: number, styleId: string): string;
    drawing_list_text_styles(handle: number): string;
    drawing_add_text_style(handle: number, requestJson: string): string;
    drawing_update_text_style(handle: number, requestJson: string): string;
    drawing_delete_text_style(handle: number, styleId: string): string;
    drawing_list_linetypes(handle: number): string;
    drawing_add_linetype(handle: number, requestJson: string): string;
    drawing_update_linetype(handle: number, requestJson: string): string;
    drawing_delete_linetype(handle: number, linetypeId: string): string;
    drawing_list_groups(handle: number): string;
    drawing_create_group(handle: number, requestJson: string): string;
    drawing_delete_group(handle: number, groupId: string): string;
    drawing_add_group_members(handle: number, requestJson: string): string;
    drawing_remove_group_members(handle: number, requestJson: string): string;
    drawing_select(handle: number, requestJson: string): string;
    drawing_snap(handle: number, requestJson: string): string;
    drawing_merge_layer(handle: number, from: string, to: string): string;
    drawing_convert_units(handle: number, unit: string): string;
    drawing_move_entity(handle: number, requestJson: string): string;
    drawing_update_entity(handle: number, requestJson: string): string;
    drawing_update_polyline(handle: number, requestJson: string): string;
    drawing_update_arc(handle: number, requestJson: string): string;
    drawing_assign_entity_layer(handle: number, requestJson: string): string;
    drawing_find_text(handle: number, requestJson: string): string;
    drawing_replace_text(handle: number, requestJson: string): string;
    drawing_transform(handle: number, requestJson: string): string;
    drawing_copy(handle: number, requestJson: string): string;
    drawing_array(handle: number, requestJson: string): string;
    drawing_explode(handle: number, entityId: string): string;
    drawing_offset(handle: number, requestJson: string): string;
    drawing_offset_by_side(handle: number, requestJson: string): string;
    drawing_trim(handle: number, requestJson: string): string;
    drawing_extend(handle: number, requestJson: string): string;
    drawing_break(handle: number, requestJson: string): string;
    drawing_fillet(handle: number, requestJson: string): string;
    drawing_chamfer(handle: number, requestJson: string): string;
    drawing_measure(handle: number, sheet?: string): string;
    drawing_measure_entities(handle: number, idsJson: string): string;
    drawing_measure_points(handle: number, pointsJson: string): string;
    drawing_ucs(handle: number, requestJson: string): string;
    drawing_stats(handle: number): string;
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
