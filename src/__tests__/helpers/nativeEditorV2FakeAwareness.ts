import type { DocumentJSON } from '../../NativeEditorBridge';
import {
    type FakeNativeEditorLocalAwarenessWireSelection,
    exactV2U32,
    type FakeNativeEditorLocalAwarenessWireIntent,
    type FakeErrorRecord,
    errorRecord,
} from './nativeEditorV2FakeRecords';
import { type FakeTransportWireConfig, type FakeSession } from './nativeEditorV2FakeTypes';
import {
    type FakeDocumentNode,
    isFakeBlockVoidNode,
    fakeAtomLabel,
    isFakeVoidNode,
    fakeScalarDocumentMap,
    cloneDoc,
} from './nativeEditorV2FakeDocument';

export function isFakeRecord(value: unknown): value is Record<string, unknown> {
    return value != null && typeof value === 'object' && !Array.isArray(value);
}

export const FAKE_AWARENESS_INTENT_KEYS = new Set(['state', 'focused', 'selection']);

export const FAKE_AWARENESS_SELECTION_KEYS = new Set(['type', 'anchor', 'head']);

export function hasFakeReservedCursor(value: unknown): boolean {
    const pending: unknown[] = [value];
    const seen = new WeakSet<object>();
    while (pending.length > 0) {
        const current = pending.pop();
        if (current == null || typeof current !== 'object') continue;
        if (seen.has(current)) continue;
        seen.add(current);
        for (const key of Reflect.ownKeys(current)) {
            if (key === 'cursor') return true;
            const descriptor = Object.getOwnPropertyDescriptor(current, key);
            if (descriptor == null || !('value' in descriptor)) return true;
            pending.push(descriptor.value);
        }
    }
    return false;
}

export function validFakeAwarenessSelection(
    value: unknown
): value is FakeNativeEditorLocalAwarenessWireSelection {
    if (!isFakeRecord(value)) return false;
    if (
        Reflect.ownKeys(value).some(
            (key) => typeof key !== 'string' || !FAKE_AWARENESS_SELECTION_KEYS.has(key)
        ) ||
        !Object.prototype.hasOwnProperty.call(value, 'type') ||
        !Object.prototype.hasOwnProperty.call(value, 'anchor') ||
        !Object.prototype.hasOwnProperty.call(value, 'head') ||
        value.type !== 'text'
    ) {
        return false;
    }
    return exactV2U32(value.anchor) != null && exactV2U32(value.head) != null;
}

export function parseFakeAwarenessIntent(
    awarenessJson: string
): FakeNativeEditorLocalAwarenessWireIntent | FakeErrorRecord {
    let parsed: unknown;
    try {
        parsed = JSON.parse(awarenessJson);
    } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        return errorRecord(
            'boundary',
            'AWARENESS_STATE_INVALID',
            `desired awareness state is not valid JSON: ${message}`
        );
    }
    if (!isFakeRecord(parsed)) {
        return errorRecord('boundary', 'AWARENESS_STATE_INVALID', 'invalid local awareness intent');
    }
    const { state, focused, selection } = parsed;
    if (
        Reflect.ownKeys(parsed).some(
            (key) => typeof key !== 'string' || !FAKE_AWARENESS_INTENT_KEYS.has(key)
        ) ||
        !isFakeRecord(state) ||
        typeof focused !== 'boolean' ||
        (selection !== undefined && selection !== null && !validFakeAwarenessSelection(selection))
    ) {
        return errorRecord('boundary', 'AWARENESS_STATE_INVALID', 'invalid local awareness intent');
    }
    if (hasFakeReservedCursor(parsed)) {
        return errorRecord(
            'boundary',
            'AWARENESS_STATE_INVALID',
            'reserved cursor key is not allowed in local awareness state'
        );
    }
    if (selection === undefined) return { state, focused };
    return {
        state,
        focused,
        selection: selection as FakeNativeEditorLocalAwarenessWireSelection | null,
    };
}

/**
 * Accept the transport intent wire record the bridge serializes. `null`
 * means "no transport"; anything else must carry a url, a connect flag, and
 * at most the static protocol-adapter descriptor.
 */
export function parseFakeTransportConfig(
    configJson: string
): FakeTransportWireConfig | FakeErrorRecord | null {
    let parsed: unknown;
    try {
        parsed = JSON.parse(configJson);
    } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        return errorRecord(
            'boundary',
            'CONFIG_INVALID',
            `collaboration transport config is not valid JSON: ${message}`
        );
    }
    if (parsed === null) return null;
    if (
        !isFakeRecord(parsed) ||
        typeof parsed.url !== 'string' ||
        parsed.url.length === 0 ||
        typeof parsed.connect !== 'boolean'
    ) {
        return errorRecord(
            'boundary',
            'CONFIG_INVALID',
            'invalid collaboration transport configuration'
        );
    }
    const descriptor = parsed.protocolAdapter;
    if (descriptor === undefined) {
        return { url: parsed.url, connect: parsed.connect };
    }
    if (
        !isFakeRecord(descriptor) ||
        !Array.isArray(descriptor.protocols) ||
        descriptor.protocols.some((protocol) => typeof protocol !== 'string')
    ) {
        return errorRecord(
            'boundary',
            'CONFIG_INVALID',
            'invalid collaboration protocol adapter descriptor'
        );
    }
    return {
        url: parsed.url,
        connect: parsed.connect,
        protocolAdapter: {
            protocols: descriptor.protocols as string[],
            ...(typeof descriptor.timeoutMillis === 'number'
                ? { timeoutMillis: descriptor.timeoutMillis }
                : {}),
            ...(Array.isArray(descriptor.terminalCloseCodes)
                ? { terminalCloseCodes: descriptor.terminalCloseCodes as number[] }
                : {}),
        },
    };
}

/**
 * Resolve the cursor one intent publishes. An omitted selection retains the
 * cursor the session already holds — the engine owns it as a sticky index,
 * so it needs no restated document position. An explicit null clears it.
 */
export function fakeCursorForIntent(
    selection: FakeNativeEditorLocalAwarenessWireSelection | null | undefined,
    retained: { anchor: number; head: number } | null
): { anchor: number; head: number } | null {
    if (selection === undefined) return retained;
    if (selection === null) return null;
    return { anchor: selection.anchor, head: selection.head };
}

export function projectFakeLocalAwareness(
    intent: FakeNativeEditorLocalAwarenessWireIntent,
    cursor: { anchor: number; head: number } | null
): {
    state: Record<string, unknown>;
    cursor: { anchor: number; head: number } | null;
} {
    const state: Record<string, unknown> = {
        state: intent.state,
        focused: intent.focused,
    };
    if (cursor != null) {
        state.cursor = {
            anchor: { type: 'fakeEngineSticky', association: 'after' },
            head: { type: 'fakeEngineSticky', association: 'after' },
        };
    }
    return {
        state,
        cursor,
    };
}

export function fakeScalarText(doc: DocumentJSON): string[] {
    const blocks = Array.isArray(doc.content) ? doc.content : [];
    return Array.from(
        blocks
            .map((rawBlock) => {
                if (rawBlock == null || typeof rawBlock !== 'object' || Array.isArray(rawBlock)) {
                    return '';
                }
                const block = rawBlock as FakeDocumentNode;
                if (isFakeBlockVoidNode(block)) return fakeAtomLabel(block);
                const inline = Array.isArray(block.content) ? block.content : [];
                return inline
                    .map((rawInline) => {
                        if (
                            rawInline == null ||
                            typeof rawInline !== 'object' ||
                            Array.isArray(rawInline)
                        ) {
                            return '';
                        }
                        const node = rawInline as FakeDocumentNode;
                        if (typeof node.text === 'string') return node.text;
                        return isFakeVoidNode(node) ? fakeAtomLabel(node) : '';
                    })
                    .join('');
            })
            .join('\n')
    );
}

export function moveFakeStickyPoint(
    position: number,
    before: DocumentJSON,
    after: DocumentJSON
): number {
    const beforeMap = fakeScalarDocumentMap(before);
    const afterMap = fakeScalarDocumentMap(after);
    const beforeText = fakeScalarText(before);
    const afterText = fakeScalarText(after);
    let prefix = 0;
    while (
        prefix < beforeText.length &&
        prefix < afterText.length &&
        beforeText[prefix] === afterText[prefix]
    ) {
        prefix += 1;
    }
    let suffix = 0;
    while (
        suffix < beforeText.length - prefix &&
        suffix < afterText.length - prefix &&
        beforeText[beforeText.length - suffix - 1] === afterText[afterText.length - suffix - 1]
    ) {
        suffix += 1;
    }
    const oldChangedEnd = beforeText.length - suffix;
    const insertedLength = afterText.length - prefix - suffix;
    const scalar = beforeMap.documentToScalar(position);
    const movedScalar =
        scalar < prefix
            ? scalar
            : scalar <= oldChangedEnd
              ? prefix + insertedLength
              : scalar + afterText.length - beforeText.length;
    return afterMap.scalarToDocument(movedScalar);
}

export function moveFakeCursorAcrossEdit(
    session: FakeSession,
    before: DocumentJSON,
    after: DocumentJSON
): void {
    if (session.localAwarenessCursor != null) {
        session.localAwarenessCursor = {
            anchor: moveFakeStickyPoint(session.localAwarenessCursor.anchor, before, after),
            head: moveFakeStickyPoint(session.localAwarenessCursor.head, before, after),
        };
    }
    session.remotePeers = session.remotePeers.map((peer) =>
        peer.cursor == null
            ? peer
            : {
                  ...peer,
                  cursor: {
                      anchor: moveFakeStickyPoint(peer.cursor.anchor, before, after),
                      head: moveFakeStickyPoint(peer.cursor.head, before, after),
                  },
              }
    );
}

export function installFakeDocument(session: FakeSession, nextDoc: DocumentJSON): void {
    const next = cloneDoc(nextDoc);
    moveFakeCursorAcrossEdit(session, session.doc, next);
    session.doc = next;
}
