import {
    type ListContext,
    type RenderElement,
    type RenderBlocksPatch,
    type Selection,
    type ActiveState,
    type HistoryState,
    type NativeEditorV2AtomicRenderSnapshot,
    type NativeEditorV2AtomicRenderPayload,
    type DocumentJSON,
    type ContentSnapshot,
} from './NativeEditorTypes';
import {
    isPlainRecord,
    hasOnlyOwnKeys,
    nativeEditorV2U32,
    RENDER_ELEMENT_TYPES,
    hasExactOwnKeys,
    validRenderMark,
    normalizeNativeEditorV2DecimalId,
    booleanRecord,
    stringArray,
    optionalBoolean,
    parseNativeEditorV2JsonValue,
    normalizeRevisionField,
    normalizeNativeEditorV2Bytes,
} from './NativeEditorResultNormalization';
import { validEditorMentionTheme } from './EditorMentionThemeValidation';

export function validListContext(value: unknown): value is ListContext {
    if (!isPlainRecord(value)) return false;
    return (
        hasOnlyOwnKeys(value, [
            'ordered',
            'index',
            'total',
            'start',
            'isFirst',
            'isLast',
            'kind',
            'checked',
        ]) &&
        typeof value.ordered === 'boolean' &&
        nativeEditorV2U32(value.index) != null &&
        nativeEditorV2U32(value.total) != null &&
        nativeEditorV2U32(value.start) != null &&
        typeof value.isFirst === 'boolean' &&
        typeof value.isLast === 'boolean' &&
        (value.kind == null || typeof value.kind === 'string') &&
        (value.checked == null || typeof value.checked === 'boolean')
    );
}

export function validRenderElement(value: unknown): value is RenderElement {
    if (!isPlainRecord(value) || !RENDER_ELEMENT_TYPES.has(value.type as RenderElement['type'])) {
        return false;
    }
    switch (value.type) {
        case 'textRun':
            return (
                hasExactOwnKeys(value, ['type', 'text', 'marks']) &&
                typeof value.text === 'string' &&
                Array.isArray(value.marks) &&
                value.marks.every(validRenderMark)
            );
        case 'blockStart':
            return (
                hasOnlyOwnKeys(value, ['type', 'nodeType', 'depth', 'listContext']) &&
                typeof value.nodeType === 'string' &&
                nativeEditorV2U32(value.depth) != null &&
                (value.listContext === undefined || validListContext(value.listContext))
            );
        case 'blockEnd':
            return hasExactOwnKeys(value, ['type']);
        case 'voidInline':
            return (
                hasOnlyOwnKeys(value, ['type', 'nodeType', 'docPos', 'attrs']) &&
                typeof value.nodeType === 'string' &&
                nativeEditorV2U32(value.docPos) != null &&
                (value.attrs === undefined || isPlainRecord(value.attrs))
            );
        case 'voidBlock':
            return (
                hasOnlyOwnKeys(value, ['type', 'nodeType', 'docPos', 'attrs', 'atomId']) &&
                typeof value.nodeType === 'string' &&
                nativeEditorV2U32(value.docPos) != null &&
                (value.attrs === undefined || isPlainRecord(value.attrs)) &&
                (value.atomId === undefined || typeof value.atomId === 'string')
            );
        case 'opaqueInlineAtom':
            return (
                hasOnlyOwnKeys(value, [
                    'type',
                    'nodeType',
                    'label',
                    'docPos',
                    'attrs',
                    'mentionTheme',
                ]) &&
                typeof value.nodeType === 'string' &&
                typeof value.label === 'string' &&
                nativeEditorV2U32(value.docPos) != null &&
                (value.attrs === undefined || isPlainRecord(value.attrs)) &&
                (value.mentionTheme === undefined || validEditorMentionTheme(value.mentionTheme))
            );
        case 'opaqueBlockAtom':
            return (
                hasOnlyOwnKeys(value, ['type', 'nodeType', 'label', 'docPos', 'attrs']) &&
                typeof value.nodeType === 'string' &&
                typeof value.label === 'string' &&
                nativeEditorV2U32(value.docPos) != null &&
                (value.attrs === undefined || isPlainRecord(value.attrs))
            );
    }
    return false;
}

export function normalizeRenderBlocks(value: unknown): RenderElement[][] | null {
    if (!Array.isArray(value)) return null;
    return value.every(
        (block) => Array.isArray(block) && block.every((element) => validRenderElement(element))
    )
        ? (value as RenderElement[][])
        : null;
}

export function normalizeRenderPatch(value: unknown): RenderBlocksPatch | null | undefined {
    if (value === null) return null;
    if (
        !isPlainRecord(value) ||
        !hasExactOwnKeys(value, [
            'baseDocumentVersion',
            'startIndex',
            'deleteCount',
            'renderBlocks',
        ])
    ) {
        return undefined;
    }
    const renderBlocks = normalizeRenderBlocks(value.renderBlocks);
    const baseDocumentVersion = normalizeNativeEditorV2DecimalId(value.baseDocumentVersion);
    const startIndex = nativeEditorV2U32(value.startIndex);
    const deleteCount = nativeEditorV2U32(value.deleteCount);
    if (
        renderBlocks == null ||
        baseDocumentVersion == null ||
        startIndex == null ||
        deleteCount == null
    ) {
        return undefined;
    }
    return { baseDocumentVersion, startIndex, deleteCount, renderBlocks };
}

export function normalizeRenderSelection(value: unknown): Selection | null {
    if (!isPlainRecord(value)) return null;
    if (value.type === 'all') return hasExactOwnKeys(value, ['type']) ? { type: 'all' } : null;
    if (value.type === 'text') {
        if (!hasExactOwnKeys(value, ['type', 'anchor', 'head', 'anchorScalar', 'headScalar'])) {
            return null;
        }
        const anchor = nativeEditorV2U32(value.anchor);
        const head = nativeEditorV2U32(value.head);
        const anchorScalar = nativeEditorV2U32(value.anchorScalar);
        const headScalar = nativeEditorV2U32(value.headScalar);
        if (anchor == null || head == null || anchorScalar == null || headScalar == null)
            return null;
        return { type: 'text', anchor, head, anchorScalar, headScalar };
    }
    if (value.type === 'node') {
        if (!hasExactOwnKeys(value, ['type', 'pos', 'posScalar'])) return null;
        const pos = nativeEditorV2U32(value.pos);
        const posScalar = nativeEditorV2U32(value.posScalar);
        if (pos == null || posScalar == null) return null;
        return { type: 'node', pos, posScalar };
    }
    return null;
}

export function normalizeRenderActiveState(value: unknown): ActiveState | null {
    if (!isPlainRecord(value)) return null;
    if (
        !hasExactOwnKeys(value, [
            'marks',
            'markAttrs',
            'nodes',
            'commands',
            'allowedMarks',
            'insertableNodes',
        ]) ||
        !booleanRecord(value.marks) ||
        !isPlainRecord(value.markAttrs) ||
        !Object.values(value.markAttrs).every(isPlainRecord) ||
        !booleanRecord(value.nodes) ||
        !booleanRecord(value.commands) ||
        !stringArray(value.allowedMarks) ||
        !stringArray(value.insertableNodes)
    ) {
        return null;
    }
    return value as unknown as ActiveState;
}

export function normalizeRenderHistoryState(value: unknown): HistoryState | null {
    if (!isPlainRecord(value) || !hasExactOwnKeys(value, ['canUndo', 'canRedo'])) return null;
    const canUndo = optionalBoolean(value.canUndo);
    const canRedo = optionalBoolean(value.canRedo);
    return canUndo == null || canRedo == null ? null : { canUndo, canRedo };
}

export function deepFreezeV2Value<T>(value: T): T {
    if (value != null && typeof value === 'object' && !Object.isFrozen(value)) {
        for (const child of Object.values(value as Record<string, unknown>)) {
            deepFreezeV2Value(child);
        }
        Object.freeze(value);
    }
    return value;
}

/** Validate and freeze the one complete render/state snapshot. */
export function normalizeNativeEditorV2RenderUpdateValue(
    value: unknown
): NativeEditorV2AtomicRenderSnapshot | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    if (
        !hasExactOwnKeys(parsed, [
            'renderBlocks',
            'renderPatch',
            'selection',
            'activeState',
            'historyState',
            'documentVersion',
            'stateRevision',
            'scalarLength',
            'documentIsEmpty',
        ])
    ) {
        return null;
    }
    const renderBlocks =
        parsed.renderBlocks === null ? null : normalizeRenderBlocks(parsed.renderBlocks);
    const renderPatch = normalizeRenderPatch(parsed.renderPatch);
    const selection = normalizeRenderSelection(parsed.selection);
    const activeState = normalizeRenderActiveState(parsed.activeState);
    const historyState = normalizeRenderHistoryState(parsed.historyState);
    const documentVersion = normalizeRevisionField(parsed, 'documentVersion');
    const stateRevision = normalizeRevisionField(parsed, 'stateRevision');
    const scalarLength = nativeEditorV2U32(parsed.scalarLength);
    const documentIsEmpty = parsed.documentIsEmpty;
    if (
        renderPatch === undefined ||
        selection == null ||
        activeState == null ||
        historyState == null ||
        documentVersion == null ||
        stateRevision == null ||
        scalarLength == null ||
        typeof documentIsEmpty !== 'boolean'
    ) {
        return null;
    }
    let renderPayload: NativeEditorV2AtomicRenderPayload;
    if (renderBlocks == null) {
        if (parsed.renderBlocks !== null || renderPatch == null) return null;
        renderPayload = { renderBlocks: null, renderPatch };
    } else {
        if (renderPatch !== null) return null;
        renderPayload = { renderBlocks, renderPatch: null };
    }
    return deepFreezeV2Value({
        ...renderPayload,
        selection,
        activeState,
        historyState,
        documentVersion,
        stateRevision,
        scalarLength,
        documentIsEmpty,
    });
}

export function normalizeNativeEditorV2DocumentJsonValue(value: unknown): DocumentJSON | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    return isPlainRecord(parsed) ? (parsed as DocumentJSON) : null;
}

export function normalizeNativeEditorV2ContentSnapshotValue(
    value: unknown
): ContentSnapshot | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed) || typeof parsed.html !== 'string' || !isPlainRecord(parsed.json)) {
        return null;
    }
    return { html: parsed.html, json: parsed.json as DocumentJSON };
}

export interface NativeEditorV2SnapshotExport {
    metadataJson: string;
    encodedState: Uint8Array;
}

/** The snapshot export record arrives as direct fields (JSON + bytes), not a JSON string. */
export function normalizeNativeEditorV2SnapshotExportValue(
    value: unknown
): NativeEditorV2SnapshotExport | null {
    if (!isPlainRecord(value) || typeof value.metadataJson !== 'string') return null;
    const encodedState = normalizeNativeEditorV2Bytes(value.encodedState);
    if (encodedState == null) return null;
    return { metadataJson: value.metadataJson, encodedState };
}

export function normalizeNativeEditorV2CreateValue(value: unknown): { editorId: string } | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    const editorId = normalizeNativeEditorV2DecimalId(parsed.editorId);
    return editorId == null ? null : { editorId };
}
