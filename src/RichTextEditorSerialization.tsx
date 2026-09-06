import { NativeEditorErrorBase, NativeEditorOperationError } from './NativeEditorBoundaryError';
import { type EditorMentionTheme } from './EditorTheme';
import {
    normalizeNativeEditorV2DecimalId,
    normalizeNativeEditorV2RenderUpdateValue,
    requireNativeEditorV2U32,
    type ActiveState,
    type NativeEditorAtomicRenderSnapshot,
    type ReadonlyActiveState,
    type Selection,
} from './NativeEditorBridge';
import { atomSelected, type AtomInstance } from './atomInstances';
import {
    type EditorToolbarFrame,
    type EditorToolbarGroupChildItem,
    type EditorToolbarIcon,
    type EditorToolbarItem,
} from './EditorToolbar';
import { IMAGE_NODE_NAME } from './schemas';
import { useRef } from 'react';
import {
    type RichTextEditorProps,
    type RemoteSelectionDecoration,
    type RichTextEditorCaretRect,
} from './RichTextEditorTypes';

export function externalCompositionErrorPayload(error: unknown): unknown {
    return error instanceof NativeEditorErrorBase ? error.error : error;
}

export const LINK_TOOLBAR_ACTION_KEY = '__native-editor-link__';

export const IMAGE_TOOLBAR_ACTION_KEY = '__native-editor-image__';

export let nextNativeEditorToolbarFrameOwnerId = 1;

export function mergeMentionSuggestionTheme(
    baseTheme: EditorMentionTheme | undefined,
    resolvedTheme: EditorMentionTheme | undefined
): EditorMentionTheme | undefined {
    if (baseTheme == null) return resolvedTheme;
    if (resolvedTheme == null) return baseTheme;

    return {
        node: { ...baseTheme.node, ...resolvedTheme.node },
        suggestions: {
            ...baseTheme.suggestions,
            ...resolvedTheme.suggestions,
            option: { ...baseTheme.suggestions?.option, ...resolvedTheme.suggestions?.option },
        },
    };
}

export function allocateToolbarFrameOwnerId(): number {
    const ownerId = nextNativeEditorToolbarFrameOwnerId;
    nextNativeEditorToolbarFrameOwnerId += 1;
    return ownerId;
}

export const EMPTY_ACTIVE_STATE: ActiveState = {
    marks: {},
    markAttrs: {},
    nodes: {},
    commands: {},
    allowedMarks: [],
    insertableNodes: [],
};

export type AtomRenderBlocks = NonNullable<NativeEditorAtomicRenderSnapshot['renderBlocks']>;

export interface AtomRenderState {
    blocks: AtomRenderBlocks;
    instanceBlocks: ReadonlyArray<ReadonlyArray<AtomInstance>>;
    instances: AtomInstance[];
    documentVersion: string | null;
    hasOnlyStableAtomKeys: boolean;
}

export function selectedAtomKeys(
    selection: Selection,
    instances: readonly AtomInstance[]
): Set<string> {
    return new Set(
        instances
            .filter((instance) => atomSelected(selection, instance.docPos))
            .map(({ key }) => key)
    );
}

export function equalStringSets(left: ReadonlySet<string>, right: ReadonlySet<string>): boolean {
    return left.size === right.size && [...left].every((value) => right.has(value));
}

export function equalAtomInstances(
    left: readonly AtomInstance[],
    right: readonly AtomInstance[]
): boolean {
    return (
        left.length === right.length &&
        left.every(
            (instance, index) =>
                instance.key === right[index]?.key &&
                instance.nodeType === right[index]?.nodeType &&
                instance.docPos === right[index]?.docPos &&
                stringifyCachedJson(instance.attrs) === stringifyCachedJson(right[index]?.attrs)
        )
    );
}

export function isRecord(value: unknown): value is Record<string, unknown> {
    return value != null && typeof value === 'object' && !Array.isArray(value);
}

export function parseSelectionFromUpdate(value: unknown): Selection | null {
    if (!isRecord(value)) return null;
    if (value.type === 'all') return { type: 'all' };
    if (value.type === 'node' && typeof value.pos === 'number') {
        return { type: 'node', pos: value.pos };
    }
    if (
        value.type === 'text' &&
        typeof value.anchor === 'number' &&
        typeof value.head === 'number'
    ) {
        return { type: 'text', anchor: value.anchor, head: value.head };
    }
    return null;
}

export function stringArray(value: unknown): string[] {
    return Array.isArray(value)
        ? value.filter((item): item is string => typeof item === 'string')
        : [];
}

export function booleanMap(value: unknown): Record<string, boolean> {
    if (!isRecord(value)) return {};
    const result: Record<string, boolean> = {};
    for (const key of Object.keys(value)) {
        if (typeof value[key] === 'boolean') result[key] = value[key] as boolean;
    }
    return result;
}

export function parseActiveStateFromUpdate(value: unknown): ActiveState | null {
    if (!isRecord(value)) return null;
    return {
        marks: booleanMap(value.marks),
        markAttrs: isRecord(value.markAttrs)
            ? (value.markAttrs as Record<string, Record<string, unknown>>)
            : {},
        nodes: booleanMap(value.nodes),
        commands: booleanMap(value.commands),
        allowedMarks: stringArray(value.allowedMarks),
        insertableNodes: stringArray(value.insertableNodes),
    };
}

export function isRevisionMismatchError(error: unknown): boolean {
    return error instanceof NativeEditorOperationError && error.code === 'REVISION_MISMATCH';
}

export function isPositionInvalidError(error: unknown): boolean {
    return error instanceof NativeEditorOperationError && error.code === 'POSITION_INVALID';
}

export interface NativeCommitPayload {
    editorId: string;
    documentRevision: string;
    updateJson: string;
}

export interface AcceptedNativeCommit {
    documentRevision: string;
    snapshot: NativeEditorAtomicRenderSnapshot;
}

/**
 * Native commits are a transport boundary, not a best-effort view hint. The
 * payload must be a complete atomic snapshot paired with the same canonical
 * revision that native says it committed.
 */
export function acceptNativeCommitPayload(
    payload: NativeCommitPayload,
    boundEditorId: string,
    lastAcceptedRevision: string | null
): AcceptedNativeCommit | null {
    const canonicalBoundEditorId = normalizeNativeEditorV2DecimalId(boundEditorId);
    const canonicalEditorId = normalizeNativeEditorV2DecimalId(payload.editorId);
    const canonicalRevision = normalizeNativeEditorV2DecimalId(payload.documentRevision);
    if (
        canonicalBoundEditorId == null ||
        canonicalBoundEditorId !== boundEditorId ||
        canonicalEditorId == null ||
        canonicalEditorId !== payload.editorId ||
        canonicalEditorId !== boundEditorId ||
        canonicalRevision == null ||
        canonicalRevision !== payload.documentRevision
    ) {
        return null;
    }
    const snapshot = normalizeNativeEditorV2RenderUpdateValue(payload.updateJson);
    if (snapshot == null || snapshot.documentVersion !== canonicalRevision) return null;
    if (lastAcceptedRevision != null && BigInt(canonicalRevision) <= BigInt(lastAcceptedRevision)) {
        return null;
    }
    return { documentRevision: canonicalRevision, snapshot };
}

export function mapToolbarChildForNative(
    item: EditorToolbarGroupChildItem,
    activeState: ReadonlyActiveState,
    editable: boolean,
    onRequestLink?: RichTextEditorProps['onRequestLink'],
    onRequestImage?: RichTextEditorProps['onRequestImage']
): EditorToolbarGroupChildItem {
    if (item.type === 'link') {
        return {
            type: 'action',
            key: LINK_TOOLBAR_ACTION_KEY,
            label: item.label,
            icon: item.icon as EditorToolbarIcon,
            buttonStyle: item.buttonStyle,
            placement: item.placement,
            isActive: activeState.marks.link === true,
            isDisabled: !editable || !onRequestLink || !activeState.allowedMarks.includes('link'),
        };
    }
    if (item.type === 'image') {
        return {
            type: 'action',
            key: IMAGE_TOOLBAR_ACTION_KEY,
            label: item.label,
            icon: item.icon as EditorToolbarIcon,
            buttonStyle: item.buttonStyle,
            placement: item.placement,
            isActive: false,
            isDisabled:
                !editable ||
                !onRequestImage ||
                !activeState.insertableNodes.includes(IMAGE_NODE_NAME),
        };
    }
    return item;
}

export function mapToolbarItemsForNative(
    items: readonly EditorToolbarItem[],
    activeState: ReadonlyActiveState,
    editable: boolean,
    onRequestLink?: RichTextEditorProps['onRequestLink'],
    onRequestImage?: RichTextEditorProps['onRequestImage']
): EditorToolbarItem[] {
    return items.map((item) => {
        if (item.type === 'group') {
            return {
                ...item,
                items: item.items.map((child) =>
                    mapToolbarChildForNative(
                        child,
                        activeState,
                        editable,
                        onRequestLink,
                        onRequestImage
                    )
                ),
            };
        }
        if (item.type === 'separator') {
            return item;
        }
        return mapToolbarChildForNative(item, activeState, editable, onRequestLink, onRequestImage);
    });
}

export function serializeRemoteSelections(
    remoteSelections?: readonly RemoteSelectionDecoration[]
): string | undefined {
    if (!remoteSelections || remoteSelections.length === 0) {
        return undefined;
    }
    const normalized = remoteSelections.map((selection) => {
        const clientId = normalizeNativeEditorV2DecimalId(selection.clientId);
        if (clientId == null) {
            throw new Error('NativeRichTextEditor: remote clientId must be canonical decimal u64');
        }
        return {
            ...selection,
            clientId,
            anchor: requireNativeEditorV2U32(selection.anchor, 'remote selection anchor'),
            head: requireNativeEditorV2U32(selection.head, 'remote selection head'),
        };
    });
    return stringifyCachedJson(normalized);
}

export function serializeToolbarFrames(
    frames: readonly EditorToolbarFrame[] | null | undefined
): string | undefined {
    if (!frames || frames.length === 0) {
        return undefined;
    }
    return JSON.stringify(frames.length === 1 ? frames[0] : { frames });
}

export function parseCaretRectJson(raw: string | null | undefined): RichTextEditorCaretRect | null {
    if (!raw) {
        return null;
    }

    try {
        const parsed = JSON.parse(raw) as Record<string, unknown>;
        const x = typeof parsed.x === 'number' ? parsed.x : null;
        const y = typeof parsed.y === 'number' ? parsed.y : null;
        const width = typeof parsed.width === 'number' ? parsed.width : null;
        const height = typeof parsed.height === 'number' ? parsed.height : null;
        const editorWidth = typeof parsed.editorWidth === 'number' ? parsed.editorWidth : null;
        const editorHeight = typeof parsed.editorHeight === 'number' ? parsed.editorHeight : null;
        if (
            x == null ||
            y == null ||
            width == null ||
            height == null ||
            editorWidth == null ||
            editorHeight == null
        ) {
            return null;
        }
        return { x, y, width, height, editorWidth, editorHeight };
    } catch {
        return null;
    }
}

export const serializedJsonCache = new WeakMap<object, string>();

export function stringifyCachedJson(value: unknown): string {
    if (value != null && typeof value === 'object') {
        const cached = serializedJsonCache.get(value);
        if (cached != null) {
            return cached;
        }
        const serialized = JSON.stringify(value);
        serializedJsonCache.set(value, serialized);
        return serialized;
    }
    return JSON.stringify(value);
}

export function useSerializedValue<T>(
    value: T | null | undefined,
    serialize: (value: T) => string | undefined,
    revision?: unknown
): string | undefined {
    const cacheRef = useRef<{
        value: T | null | undefined;
        revision: unknown;
        hasRevision: boolean;
        serialized: string | undefined;
    } | null>(null);
    const hasRevision = revision !== undefined;
    const cached = cacheRef.current;

    if (cached) {
        if (hasRevision && cached.hasRevision && Object.is(cached.revision, revision)) {
            return cached.serialized;
        }
        if (Object.is(cached.value, value) && cached.hasRevision === hasRevision) {
            return cached.serialized;
        }
    }

    const serialized = value == null ? undefined : serialize(value);
    cacheRef.current = {
        value,
        revision,
        hasRevision,
        serialized,
    };
    return serialized;
}
