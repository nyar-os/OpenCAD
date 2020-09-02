/**
 * File-backed undo history for `history: 'sidecar'` sessions.
 *
 * The native store keeps history per handle, and a CLI invocation opens a fresh
 * handle, so a separate process cannot see an earlier revision. This module
 * keeps the previous document beside the project file in
 * `<file>.history.json` so undo and redo survive across processes.
 *
 * It is loaded through a dynamic `import()` only when a session declares
 * `history: 'sidecar'`; the default memory session never touches the disk here.
 */

import fs from 'node:fs';

/** Maximum number of snapshots kept in one direction. */
export const HISTORY_LIMIT = 32;

/** On-disk shape of a sidecar file. */
export interface SidecarState {
    readonly version: number;
    undo: string[];
    redo: string[];
}

const HISTORY_VERSION = 1;

/** Path of the sidecar that belongs to one project file. */
export function sidecarPath(file: string): string {
    return `${file}.history.json`;
}

/** Removes the sidecar of one project file, if any. */
export function clearSidecarHistory(file: string): void {
    fs.rmSync(sidecarPath(file), { force: true });
}

/** One project file and the snapshots recorded beside it. */
export class SidecarHistory {
    /** Project file this history belongs to. */
    readonly file: string;

    constructor(file: string) {
        this.file = file;
    }

    /** Reads the current project content that a mutation is about to replace. */
    read(): string {
        return fs.readFileSync(this.file, 'utf8');
    }

    /** Pushes `previous` onto the undo stack and drops the redo stack. */
    record(previous: string): void {
        const history = this.load();
        history.undo.push(previous);
        if (history.undo.length > HISTORY_LIMIT) {
            history.undo.splice(0, history.undo.length - HISTORY_LIMIT);
        }
        history.redo.length = 0;
        this.store(history);
    }

    /** Number of snapshots available in each direction. */
    counts(): { readonly undo: number; readonly redo: number } {
        const history = this.load();
        return { undo: history.undo.length, redo: history.redo.length };
    }

    /**
     * Swaps the project file with the newest snapshot in one direction and
     * bumps the restored revision relative to the file it replaces. Returns
     * `undefined` when that direction has no snapshot.
     */
    swap(direction: 'undo' | 'redo'): string | undefined {
        const history = this.load();
        const source = direction === 'undo' ? history.undo : history.redo;
        const destination = direction === 'undo' ? history.redo : history.undo;
        const snapshot = source.pop();
        if (snapshot === undefined) {
            return undefined;
        }
        const current = fs.readFileSync(this.file, 'utf8');
        const restored = JSON.parse(snapshot) as { document?: { revision?: unknown } };
        if (restored.document !== undefined) {
            restored.document.revision = revisionOf(current) + 1;
        }
        const content = `${JSON.stringify(restored, null, 2)}\n`;
        writeAtomic(this.file, content);
        destination.push(current);
        if (destination.length > HISTORY_LIMIT) {
            destination.splice(0, destination.length - HISTORY_LIMIT);
        }
        this.store(history);
        return content;
    }

    private load(): SidecarState {
        try {
            const parsed = JSON.parse(fs.readFileSync(sidecarPath(this.file), 'utf8')) as Partial<SidecarState>;
            if (parsed.version === HISTORY_VERSION && Array.isArray(parsed.undo) && Array.isArray(parsed.redo)) {
                return {
                    version: HISTORY_VERSION,
                    undo: parsed.undo.filter((item): item is string => typeof item === 'string'),
                    redo: parsed.redo.filter((item): item is string => typeof item === 'string'),
                };
            }
        } catch {
            // No history yet, or a sidecar this build cannot read.
        }
        return { version: HISTORY_VERSION, undo: [], redo: [] };
    }

    private store(history: SidecarState): void {
        writeAtomic(sidecarPath(this.file), `${JSON.stringify(history)}\n`);
    }
}

/** Reads the committed revision from a `.ocad` envelope. */
function revisionOf(document: string): number {
    const parsed = JSON.parse(document) as { document?: { revision?: unknown } };
    const revision = parsed.document?.revision;
    return typeof revision === 'number' ? revision : 1;
}

function writeAtomic(target: string, content: string): void {
    const temporary = `${target}.${process.pid}.tmp`;
    fs.writeFileSync(temporary, content);
    fs.renameSync(temporary, target);
}
