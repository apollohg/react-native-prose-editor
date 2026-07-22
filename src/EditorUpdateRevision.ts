import type { NativeEditorV2Error } from './NativeEditorBoundaryError';

const MAX_EDITOR_UPDATE_REVISION = 0xffff_ffff;

export type EditorUpdateRevisionAllocation = { revision: number } | { error: NativeEditorV2Error };

/** Allocate the u32 revision carried by the native view update prop. */
export function allocateEditorUpdateRevision(
    currentRevision: number
): EditorUpdateRevisionAllocation {
    if (currentRevision >= MAX_EDITOR_UPDATE_REVISION) {
        return {
            error: {
                domain: 'boundary',
                code: 'CONFIG_INVALID',
                message: 'NativeRichTextEditor: editor update revision counter exhausted',
                requestId: null,
                operationIndex: null,
                limit: String(MAX_EDITOR_UPDATE_REVISION),
                actual: String(MAX_EDITOR_UPDATE_REVISION + 1),
                details: null,
            },
        };
    }
    return { revision: currentRevision + 1 };
}
