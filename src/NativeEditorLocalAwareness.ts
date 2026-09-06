import { NativeEditorV2BoundaryError } from './NativeEditorBoundaryError';
import { invalidV2RequestError, nativeEditorV2U32 } from './NativeEditorResultNormalization';
import {
    type NativeEditorLocalAwarenessSelection,
    NativeEditorLocalAwarenessSelectionValue,
} from './NativeEditorTypes';
import { normalizeV2JsonValue, serializeV2CreateEnvelope } from './NativeEditorCreateJson';

export const LOCAL_AWARENESS_INTENT_KEYS = new Set(['state', 'focused', 'selection']);

export const LOCAL_AWARENESS_SELECTION_VALUES = new WeakMap<
    object,
    Readonly<{ anchor: number; head: number }>
>();

export function invalidLocalAwarenessIntent(message = 'invalid local awareness intent'): never {
    throw invalidV2RequestError(`NativeEditorBridge: ${message}`);
}

/**
 * Create the only caller-owned local-awareness selection accepted at the
 * JavaScript-to-native boundary. The private WeakMap makes provenance an API
 * capability instead of a structural object-shape check.
 */
export function createNativeEditorLocalAwarenessSelection(
    anchor: number,
    head: number
): NativeEditorLocalAwarenessSelection {
    const acceptedAnchor = nativeEditorV2U32(anchor);
    const acceptedHead = nativeEditorV2U32(head);
    if (acceptedAnchor == null || acceptedHead == null) invalidLocalAwarenessIntent();

    const selection: NativeEditorLocalAwarenessSelection =
        new NativeEditorLocalAwarenessSelectionValue(acceptedAnchor, acceptedHead);
    Object.freeze(selection);
    LOCAL_AWARENESS_SELECTION_VALUES.set(selection, selection);
    return selection;
}

export function isLocalAwarenessRecord(value: unknown): value is Record<string, unknown> {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return false;
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
}

export function localAwarenessOwnDataValue(record: Record<string, unknown>, key: string): unknown {
    const descriptor = Object.getOwnPropertyDescriptor(record, key);
    if (descriptor === undefined || !('value' in descriptor) || descriptor.enumerable !== true) {
        invalidLocalAwarenessIntent();
    }
    return descriptor.value;
}

export function validateLocalAwarenessSelection(selection: unknown): {
    anchor: number;
    head: number;
} {
    if (selection == null || typeof selection !== 'object') invalidLocalAwarenessIntent();

    // Do not inspect caller-held values before provenance succeeds: a Proxy
    // can imitate every structural and own-data check, but it cannot inherit
    // this module's WeakMap identity.
    const factoryValue = LOCAL_AWARENESS_SELECTION_VALUES.get(selection);
    if (factoryValue === undefined) invalidLocalAwarenessIntent();

    const anchor = nativeEditorV2U32(factoryValue.anchor);
    const head = nativeEditorV2U32(factoryValue.head);
    if (anchor == null || head == null) {
        invalidLocalAwarenessIntent();
    }
    return { anchor, head };
}

/** Reject caller-owned sticky cursor data before a native call can occur. */
export function rejectReservedAwarenessCursor(value: unknown): void {
    const pending: unknown[] = [value];
    const seen = new WeakSet<object>();
    while (pending.length > 0) {
        const current = pending.pop();
        if (current == null || typeof current !== 'object') continue;
        if (seen.has(current)) continue;
        seen.add(current);
        for (const key of Reflect.ownKeys(current)) {
            if (key === 'cursor') invalidLocalAwarenessIntent('reserved cursor key is not allowed');
            const descriptor = Object.getOwnPropertyDescriptor(current, key);
            if (descriptor == null || !('value' in descriptor)) invalidLocalAwarenessIntent();
            pending.push(descriptor.value);
        }
    }
}

export function normalizeLocalAwarenessState(value: unknown): Record<string, unknown> {
    try {
        const normalized = normalizeV2JsonValue(value, 'local awareness state', {
            seen: new WeakSet<object>(),
            work: 0,
        });
        if (!isLocalAwarenessRecord(normalized) || Object.getPrototypeOf(normalized) !== null) {
            invalidLocalAwarenessIntent();
        }
        rejectReservedAwarenessCursor(normalized);
        return normalized;
    } catch (error) {
        if (error instanceof NativeEditorV2BoundaryError) throw error;
        invalidLocalAwarenessIntent();
    }
}

export interface NativeEditorLocalAwarenessWireIntent {
    state: Record<string, unknown>;
    focused: boolean;
    /** Absent retains the Rust-owned cursor; `null` clears it. */
    selection?: { type: 'text'; anchor: number; head: number } | null;
}

export function validateLocalAwarenessIntent(
    intent: unknown
): NativeEditorLocalAwarenessWireIntent {
    try {
        if (
            !isLocalAwarenessRecord(intent) ||
            Reflect.ownKeys(intent).some(
                (key) => typeof key !== 'string' || !LOCAL_AWARENESS_INTENT_KEYS.has(key)
            ) ||
            !Object.prototype.hasOwnProperty.call(intent, 'state') ||
            !Object.prototype.hasOwnProperty.call(intent, 'focused')
        ) {
            invalidLocalAwarenessIntent();
        }
        const state = normalizeLocalAwarenessState(localAwarenessOwnDataValue(intent, 'state'));
        const focused = localAwarenessOwnDataValue(intent, 'focused');
        if (typeof focused !== 'boolean') invalidLocalAwarenessIntent();

        if (!Object.prototype.hasOwnProperty.call(intent, 'selection')) {
            // Absent: retain whatever cursor Rust already holds.
            return { state, focused };
        }
        const rawSelection = localAwarenessOwnDataValue(intent, 'selection');
        if (rawSelection === null) return { state, focused, selection: null };
        const selection = validateLocalAwarenessSelection(rawSelection);
        return { state, focused, selection: { type: 'text', ...selection } };
    } catch (error) {
        if (error instanceof NativeEditorV2BoundaryError) throw error;
        invalidLocalAwarenessIntent();
    }
}

export function serializeLocalAwarenessIntent(
    intent: NativeEditorLocalAwarenessWireIntent
): string {
    try {
        const wire = Object.create(null) as Record<string, unknown>;
        wire.state = intent.state;
        wire.focused = intent.focused;
        if (intent.selection === null) {
            wire.selection = null;
        } else if (intent.selection !== undefined) {
            const selection = Object.create(null) as Record<string, unknown>;
            selection.type = intent.selection.type;
            selection.anchor = intent.selection.anchor;
            selection.head = intent.selection.head;
            wire.selection = selection;
        }
        return serializeV2CreateEnvelope(wire);
    } catch {
        invalidLocalAwarenessIntent();
    }
}
