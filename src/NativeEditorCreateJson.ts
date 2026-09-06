import {
    invalidV2CreateRequestError,
    V2_CREATE_JSON_MAX_WORK,
    V2_CREATE_JSON_MAX_BYTES,
    V2_CREATE_WIRE_MAX_BYTES,
    V2_CREATE_STRING_CHAR_CODE_AT,
    V2_CREATE_NUMBER_TO_STRING,
    V2_CREATE_JSON_MAX_DEPTH,
    isV2CreateRecord,
    emptyV2CreateRecord,
    V2_CREATE_JSON_OUTPUT_CHUNK_SIZE,
    V2_CREATE_STRING_SLICE,
    V2_CREATE_ENVELOPE_JSON_MAX_DEPTH,
} from './NativeEditorCreateValidation';

export function invalidV2JsonValue(label: string): never {
    throw invalidV2CreateRequestError(`NativeEditorBridge: invalid ${label} for v2 create`);
}

export interface V2JsonNormalizationTraversal {
    readonly seen: WeakSet<object>;
    work: number;
}

export interface V2JsonNormalizationBudget {
    bytes: number;
}

export type V2JsonNormalizationTarget = Record<string, unknown> | unknown[] | null;

export interface V2JsonNormalizationValueFrame {
    readonly type: 'value';
    readonly value: unknown;
    readonly depth: number;
    readonly target: V2JsonNormalizationTarget;
    readonly key: string | number | null;
}

export interface V2JsonNormalizationArrayFrame {
    readonly type: 'array';
    readonly value: unknown[];
    readonly normalized: unknown[];
    readonly keys: readonly PropertyKey[];
    readonly length: number;
    readonly depth: number;
    readonly nextKeyIndex: number;
    readonly elementCount: number;
}

export interface V2JsonNormalizationObjectFrame {
    readonly type: 'object';
    readonly value: Record<string, unknown>;
    readonly normalized: Record<string, unknown>;
    readonly keys: readonly PropertyKey[];
    readonly depth: number;
    readonly nextKeyIndex: number;
    readonly fieldCount: number;
}

export type V2JsonNormalizationFrame =
    | V2JsonNormalizationValueFrame
    | V2JsonNormalizationArrayFrame
    | V2JsonNormalizationObjectFrame;

export interface V2JsonSerializationState {
    bytes: number;
    work: number;
}

export interface V2JsonSerializationValueFrame {
    readonly type: 'value';
    readonly value: unknown;
    readonly depth: number;
}

export interface V2JsonSerializationArrayFrame {
    readonly type: 'array';
    readonly value: unknown[];
    readonly length: number;
    readonly depth: number;
    readonly index: number;
}

export interface V2JsonSerializationObjectFrame {
    readonly type: 'object';
    readonly value: Record<string, unknown>;
    readonly keys: readonly string[];
    readonly depth: number;
    readonly index: number;
}

export type V2JsonSerializationFrame =
    | V2JsonSerializationValueFrame
    | V2JsonSerializationArrayFrame
    | V2JsonSerializationObjectFrame;

export function chargeV2JsonWork(state: V2JsonNormalizationTraversal, label: string): void {
    if (state.work >= V2_CREATE_JSON_MAX_WORK) invalidV2JsonValue(label);
    state.work += 1;
}

export function chargeV2JsonBytes(
    budget: V2JsonNormalizationBudget,
    amount: number,
    label: string
): void {
    if (
        !Number.isSafeInteger(amount) ||
        amount < 0 ||
        amount > V2_CREATE_JSON_MAX_BYTES - budget.bytes
    ) {
        invalidV2JsonValue(label);
    }
    budget.bytes += amount;
}

export function chargeV2JsonSerializationWork(
    state: V2JsonSerializationState,
    label: string
): void {
    if (state.work >= V2_CREATE_WIRE_MAX_BYTES) invalidV2JsonValue(label);
    state.work += 1;
}

export function chargeV2JsonSerializationBytes(
    state: V2JsonSerializationState,
    amount: number,
    label: string
): void {
    if (
        !Number.isSafeInteger(amount) ||
        amount < 0 ||
        amount > V2_CREATE_WIRE_MAX_BYTES - state.bytes
    ) {
        invalidV2JsonValue(label);
    }
    state.bytes += amount;
}

export function utf8V2JsonByteLength(value: string): number {
    let bytes = 0;
    for (let index = 0; index < value.length; index += 1) {
        const code = V2_CREATE_STRING_CHAR_CODE_AT.call(value, index);
        if (code <= 0x7f) {
            bytes += 1;
        } else if (code <= 0x7ff) {
            bytes += 2;
        } else if (code >= 0xd800 && code <= 0xdbff) {
            const next =
                index + 1 < value.length
                    ? V2_CREATE_STRING_CHAR_CODE_AT.call(value, index + 1)
                    : -1;
            if (next >= 0xdc00 && next <= 0xdfff) {
                bytes += 4;
                index += 1;
            } else {
                bytes += 3;
            }
        } else {
            bytes += 3;
        }
    }
    return bytes;
}

export function serializeV2JsonNumber(value: number, label: string): string {
    if (!Number.isFinite(value)) invalidV2JsonValue(label);
    return value === 0 ? '0' : V2_CREATE_NUMBER_TO_STRING.call(value);
}

export function chargeV2JsonStringBytes(
    value: string,
    budget: V2JsonNormalizationBudget,
    label: string
): void {
    chargeV2JsonBytes(budget, 2, label);
    for (let index = 0; index < value.length; index += 1) {
        const code = V2_CREATE_STRING_CHAR_CODE_AT.call(value, index);
        if (code === 0x22 || code === 0x5c || code === 0x08 || code === 0x09) {
            chargeV2JsonBytes(budget, 2, label);
        } else if (code === 0x0a || code === 0x0c || code === 0x0d) {
            chargeV2JsonBytes(budget, 2, label);
        } else if (code <= 0x1f) {
            chargeV2JsonBytes(budget, 6, label);
        } else if (code <= 0x7f) {
            chargeV2JsonBytes(budget, 1, label);
        } else if (code <= 0x7ff) {
            chargeV2JsonBytes(budget, 2, label);
        } else if (code >= 0xd800 && code <= 0xdbff) {
            const next =
                index + 1 < value.length
                    ? V2_CREATE_STRING_CHAR_CODE_AT.call(value, index + 1)
                    : -1;
            if (next >= 0xdc00 && next <= 0xdfff) {
                chargeV2JsonBytes(budget, 4, label);
                index += 1;
            } else {
                chargeV2JsonBytes(budget, 6, label);
            }
        } else if (code >= 0xdc00 && code <= 0xdfff) {
            chargeV2JsonBytes(budget, 6, label);
        } else {
            chargeV2JsonBytes(budget, 3, label);
        }
    }
}

export function normalizeV2JsonValue(
    value: unknown,
    label: string,
    traversal: V2JsonNormalizationTraversal,
    budget: V2JsonNormalizationBudget = { bytes: 0 }
): unknown {
    let normalizedRoot: unknown;
    const frames: V2JsonNormalizationFrame[] = [
        { type: 'value', value, depth: 0, target: null, key: null },
    ];

    const installNormalizedValue = (
        target: V2JsonNormalizationTarget,
        key: string | number | null,
        normalized: unknown
    ): void => {
        if (target === null) {
            normalizedRoot = normalized;
            return;
        }
        if (key === null) invalidV2JsonValue(label);
        Object.defineProperty(target, key, {
            configurable: true,
            enumerable: true,
            value: normalized,
            writable: true,
        });
    };

    while (frames.length > 0) {
        const frame = frames.pop();
        if (frame === undefined) invalidV2JsonValue(label);

        if (frame.type === 'array') {
            if (frame.nextKeyIndex === frame.keys.length) {
                if (frame.elementCount !== frame.length) invalidV2JsonValue(label);
                Object.setPrototypeOf(frame.normalized, null);
                continue;
            }
            const key = frame.keys[frame.nextKeyIndex];
            if (key === 'length') {
                frames.push({ ...frame, nextKeyIndex: frame.nextKeyIndex + 1 });
                continue;
            }
            if (typeof key !== 'string' || !/^(0|[1-9]\d*)$/.test(key)) {
                invalidV2JsonValue(label);
            }
            const index = Number(key);
            if (
                !Number.isSafeInteger(index) ||
                index !== frame.elementCount ||
                index < 0 ||
                index >= frame.length
            ) {
                invalidV2JsonValue(label);
            }
            const descriptor = Object.getOwnPropertyDescriptor(frame.value, key);
            if (
                descriptor === undefined ||
                !('value' in descriptor) ||
                descriptor.enumerable !== true
            ) {
                invalidV2JsonValue(label);
            }
            if (frame.elementCount > 0) chargeV2JsonBytes(budget, 1, label);
            frames.push({
                ...frame,
                nextKeyIndex: frame.nextKeyIndex + 1,
                elementCount: frame.elementCount + 1,
            });
            frames.push({
                type: 'value',
                value: descriptor.value,
                depth: frame.depth,
                target: frame.normalized,
                key: frame.elementCount,
            });
            continue;
        }

        if (frame.type === 'object') {
            if (frame.nextKeyIndex === frame.keys.length) continue;
            const key = frame.keys[frame.nextKeyIndex];
            if (typeof key !== 'string') invalidV2JsonValue(label);
            const descriptor = Object.getOwnPropertyDescriptor(frame.value, key);
            if (
                descriptor === undefined ||
                !('value' in descriptor) ||
                descriptor.enumerable !== true
            ) {
                invalidV2JsonValue(label);
            }
            if (descriptor.value === undefined) {
                frames.push({
                    ...frame,
                    nextKeyIndex: frame.nextKeyIndex + 1,
                });
                continue;
            }
            if (frame.fieldCount > 0) chargeV2JsonBytes(budget, 1, label);
            chargeV2JsonStringBytes(key, budget, label);
            chargeV2JsonBytes(budget, 1, label);
            frames.push({
                ...frame,
                nextKeyIndex: frame.nextKeyIndex + 1,
                fieldCount: frame.fieldCount + 1,
            });
            frames.push({
                type: 'value',
                value: descriptor.value,
                depth: frame.depth,
                target: frame.normalized,
                key,
            });
            continue;
        }

        if (frame.depth > V2_CREATE_JSON_MAX_DEPTH) invalidV2JsonValue(label);
        chargeV2JsonWork(traversal, label);
        if (frame.value === null) {
            chargeV2JsonBytes(budget, 4, label);
            installNormalizedValue(frame.target, frame.key, frame.value);
            continue;
        }
        if (typeof frame.value === 'string') {
            chargeV2JsonStringBytes(frame.value, budget, label);
            installNormalizedValue(frame.target, frame.key, frame.value);
            continue;
        }
        if (typeof frame.value === 'boolean') {
            chargeV2JsonBytes(budget, frame.value ? 4 : 5, label);
            installNormalizedValue(frame.target, frame.key, frame.value);
            continue;
        }
        if (typeof frame.value === 'number') {
            const serialized = serializeV2JsonNumber(frame.value, label);
            chargeV2JsonBytes(budget, serialized.length, label);
            installNormalizedValue(frame.target, frame.key, frame.value);
            continue;
        }
        if (typeof frame.value !== 'object') invalidV2JsonValue(label);

        if (traversal.seen.has(frame.value)) invalidV2JsonValue(label);
        traversal.seen.add(frame.value);
        chargeV2JsonBytes(budget, 2, label);
        const childDepth = frame.depth + 1;
        if (Array.isArray(frame.value)) {
            const prototype = Object.getPrototypeOf(frame.value);
            if (prototype !== Array.prototype && prototype !== null) invalidV2JsonValue(label);
            const lengthDescriptor = Object.getOwnPropertyDescriptor(frame.value, 'length');
            if (
                lengthDescriptor === undefined ||
                !('value' in lengthDescriptor) ||
                typeof lengthDescriptor.value !== 'number'
            ) {
                invalidV2JsonValue(label);
            }
            const normalized: unknown[] = [];
            installNormalizedValue(frame.target, frame.key, normalized);
            frames.push({
                type: 'array',
                value: frame.value,
                normalized,
                keys: Reflect.ownKeys(frame.value),
                length: lengthDescriptor.value,
                depth: childDepth,
                nextKeyIndex: 0,
                elementCount: 0,
            });
            continue;
        }

        if (!isV2CreateRecord(frame.value)) invalidV2JsonValue(label);
        const normalized = emptyV2CreateRecord();
        installNormalizedValue(frame.target, frame.key, normalized);
        frames.push({
            type: 'object',
            value: frame.value,
            normalized,
            keys: Reflect.ownKeys(frame.value),
            depth: childDepth,
            nextKeyIndex: 0,
            fieldCount: 0,
        });
    }

    if (normalizedRoot === undefined) invalidV2JsonValue(label);
    return normalizedRoot;
}

export class V2JsonSerializationWriter {
    private readonly _chunks: string[] = [];
    private _current = '';

    append(value: string, state: V2JsonSerializationState, label: string): void {
        chargeV2JsonSerializationBytes(state, utf8V2JsonByteLength(value), label);
        this._current += value;
        if (this._current.length >= V2_CREATE_JSON_OUTPUT_CHUNK_SIZE) {
            this._chunks.push(this._current);
            this._current = '';
        }
    }

    finish(): string {
        if (this._current.length > 0) this._chunks.push(this._current);
        return this._chunks.join('');
    }
}

export function appendV2JsonString(
    writer: V2JsonSerializationWriter,
    value: string,
    state: V2JsonSerializationState,
    label: string
): void {
    writer.append('"', state, label);
    let segmentStart = 0;
    for (let index = 0; index < value.length; index += 1) {
        chargeV2JsonSerializationWork(state, label);
        const code = V2_CREATE_STRING_CHAR_CODE_AT.call(value, index);
        let escape: string | undefined;
        switch (code) {
            case 0x08:
                escape = '\\b';
                break;
            case 0x09:
                escape = '\\t';
                break;
            case 0x0a:
                escape = '\\n';
                break;
            case 0x0c:
                escape = '\\f';
                break;
            case 0x0d:
                escape = '\\r';
                break;
            case 0x22:
                escape = '\\"';
                break;
            case 0x5c:
                escape = '\\\\';
                break;
            default:
                if (code <= 0x1f) {
                    escape = `\\u00${code.toString(16).padStart(2, '0')}`;
                } else if (code >= 0xd800 && code <= 0xdbff) {
                    const next =
                        index + 1 < value.length
                            ? V2_CREATE_STRING_CHAR_CODE_AT.call(value, index + 1)
                            : -1;
                    if (next >= 0xdc00 && next <= 0xdfff) {
                        chargeV2JsonSerializationWork(state, label);
                        index += 1;
                    } else {
                        escape = `\\u${code.toString(16).padStart(4, '0')}`;
                    }
                } else if (code >= 0xdc00 && code <= 0xdfff) {
                    escape = `\\u${code.toString(16).padStart(4, '0')}`;
                }
                break;
        }
        if (escape === undefined) continue;
        if (segmentStart < index) {
            writer.append(V2_CREATE_STRING_SLICE.call(value, segmentStart, index), state, label);
        }
        writer.append(escape, state, label);
        segmentStart = index + 1;
    }
    if (segmentStart < value.length) {
        writer.append(V2_CREATE_STRING_SLICE.call(value, segmentStart), state, label);
    }
    writer.append('"', state, label);
}

export function serializeV2CreateEnvelope(value: Record<string, unknown>): string {
    const label = 'v2 create config';
    const writer = new V2JsonSerializationWriter();
    const state: V2JsonSerializationState = { bytes: 0, work: 0 };
    const frames: V2JsonSerializationFrame[] = [{ type: 'value', value, depth: 0 }];

    while (frames.length > 0) {
        const frame = frames.pop();
        if (frame === undefined) invalidV2JsonValue(label);

        if (frame.type === 'array') {
            if (frame.index === frame.length) {
                writer.append(']', state, label);
                continue;
            }
            if (frame.index > 0) writer.append(',', state, label);
            const descriptor = Object.getOwnPropertyDescriptor(frame.value, String(frame.index));
            if (
                descriptor === undefined ||
                !('value' in descriptor) ||
                descriptor.enumerable !== true
            ) {
                invalidV2JsonValue(label);
            }
            frames.push({ ...frame, index: frame.index + 1 });
            frames.push({ type: 'value', value: descriptor.value, depth: frame.depth + 1 });
            continue;
        }

        if (frame.type === 'object') {
            if (frame.index === frame.keys.length) {
                writer.append('}', state, label);
                continue;
            }
            if (frame.index > 0) writer.append(',', state, label);
            const key = frame.keys[frame.index];
            const descriptor = Object.getOwnPropertyDescriptor(frame.value, key);
            if (
                descriptor === undefined ||
                !('value' in descriptor) ||
                descriptor.enumerable !== true
            ) {
                invalidV2JsonValue(label);
            }
            appendV2JsonString(writer, key, state, label);
            writer.append(':', state, label);
            frames.push({ ...frame, index: frame.index + 1 });
            frames.push({ type: 'value', value: descriptor.value, depth: frame.depth + 1 });
            continue;
        }

        if (frame.depth > V2_CREATE_ENVELOPE_JSON_MAX_DEPTH) invalidV2JsonValue(label);
        chargeV2JsonSerializationWork(state, label);
        if (frame.value === null) {
            writer.append('null', state, label);
        } else if (typeof frame.value === 'string') {
            appendV2JsonString(writer, frame.value, state, label);
        } else if (typeof frame.value === 'boolean') {
            writer.append(frame.value ? 'true' : 'false', state, label);
        } else if (typeof frame.value === 'number') {
            writer.append(serializeV2JsonNumber(frame.value, label), state, label);
        } else if (Array.isArray(frame.value)) {
            if (Object.getPrototypeOf(frame.value) !== null) invalidV2JsonValue(label);
            const lengthDescriptor = Object.getOwnPropertyDescriptor(frame.value, 'length');
            if (
                lengthDescriptor === undefined ||
                !('value' in lengthDescriptor) ||
                typeof lengthDescriptor.value !== 'number'
            ) {
                invalidV2JsonValue(label);
            }
            writer.append('[', state, label);
            frames.push({
                type: 'array',
                value: frame.value,
                length: lengthDescriptor.value,
                depth: frame.depth,
                index: 0,
            });
        } else if (isV2CreateRecord(frame.value) && Object.getPrototypeOf(frame.value) === null) {
            const keys = Reflect.ownKeys(frame.value);
            if (keys.some((key) => typeof key !== 'string')) invalidV2JsonValue(label);
            writer.append('{', state, label);
            frames.push({
                type: 'object',
                value: frame.value,
                keys: keys as string[],
                depth: frame.depth,
                index: 0,
            });
        } else {
            invalidV2JsonValue(label);
        }
    }

    return writer.finish();
}
