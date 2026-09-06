import { HARD_EDITOR_RESOURCE_LIMITS, validateEditorCreateLimits } from './ResourceLimits';
import {
    NativeEditorBoundaryError,
    NativeEditorV2BoundaryError,
} from './NativeEditorBoundaryError';
import { type NativeEditorV2CreateConfig } from './NativeEditorTypes';

export const V2_CREATE_CONFIG_KEYS = new Set([
    'initialization',
    'schema',
    'fragmentName',
    'policy',
    'limits',
]);

export const V2_CREATE_POLICY_KEYS = new Set([
    'maxLength',
    'readOnly',
    'inputFilter',
    'allowBase64Images',
]);

export const V2_CREATE_LIMIT_KEYS = new Set(['resource', 'editing', 'collaboration']);

export const V2_CREATE_RESOURCE_LIMIT_KEYS = new Set([
    'maxInputBytes',
    'maxDocumentNodes',
    'maxDocumentDepth',
    'maxSchemaNodes',
    'maxSchemaExpressionBytes',
    'maxCollaborationMessageBytes',
    'maxEncodedStateBytes',
]);

export const V2_CREATE_EDITING_LIMIT_KEYS = new Set([
    'maxOperationsPerTransaction',
    'maxUndoGroups',
    'maxUndoRetainedUnits',
    'maxDerivedOutputBytes',
]);

export const V2_CREATE_COLLABORATION_LIMIT_KEYS = new Set([
    'maxFramesPerMessage',
    'maxFrameBytes',
    'maxAggregateResponseBytes',
    'maxAwarenessPeers',
    'maxAwarenessPeerBytes',
    'maxAwarenessBytes',
    'maxPendingOutboxMessages',
    'maxPendingOutboxBytes',
    'maxPendingDependencyUpdateBytes',
    'maxPendingDependencyUpdateWork',
]);

export const V2_CREATE_INITIALIZATION_KEYS: Readonly<Record<string, ReadonlySet<string>>> = {
    localEmpty: new Set(['type']),
    localJson: new Set(['type', 'json']),
    localHtml: new Set(['type', 'html']),
    room: new Set(['type', 'documentId', 'lineageId', 'snapshot']),
};

export const V2_CREATE_ROOM_SNAPSHOT_KEYS = new Set(['metadata', 'encodedState']);

export const V2_CREATE_SNAPSHOT_METADATA_KEYS = new Set([
    'formatVersion',
    'documentId',
    'lineageId',
    'fragmentName',
    'schemaFingerprint',
]);

export const V2_CREATE_MAX_U32 = 0xffff_ffff;

export const V2_CREATE_JSON_MAX_BYTES = HARD_EDITOR_RESOURCE_LIMITS.maxInputBytes;

export const V2_CREATE_JSON_MAX_DEPTH = HARD_EDITOR_RESOURCE_LIMITS.maxDocumentDepth * 2 + 16;

export const V2_CREATE_JSON_MAX_WORK = HARD_EDITOR_RESOURCE_LIMITS.maxInputBytes;

export const V2_CREATE_ENVELOPE_MAX_BYTES = 64 * 1024;

export const V2_CREATE_WIRE_MAX_BYTES =
    V2_CREATE_JSON_MAX_BYTES * 7 + V2_CREATE_ENVELOPE_MAX_BYTES + 2;

export const V2_CREATE_ENVELOPE_JSON_MAX_DEPTH = V2_CREATE_JSON_MAX_DEPTH + 8;

export const V2_CREATE_JSON_OUTPUT_CHUNK_SIZE = 64 * 1024;

export const V2_CREATE_STRING_CHAR_CODE_AT = String.prototype.charCodeAt;

export const V2_CREATE_STRING_SLICE = String.prototype.slice;

export const V2_CREATE_NUMBER_TO_STRING = Number.prototype.toString;

export class NativeEditorV2CreateConfigError extends Error {
    constructor(message: string) {
        super(message);
        this.name = 'NativeEditorV2CreateConfigError';
    }
}

export function invalidV2CreateRequestError(message: string): NativeEditorV2CreateConfigError {
    return new NativeEditorV2CreateConfigError(message);
}

export function validateV2CreateLimits(limits: NativeEditorV2CreateConfig['limits']): void {
    try {
        validateEditorCreateLimits(limits);
    } catch (error) {
        if (!(error instanceof NativeEditorBoundaryError)) throw error;
        throw new NativeEditorV2BoundaryError({
            domain: 'boundary',
            code: error.code,
            message: error.message,
            requestId: null,
            operationIndex: null,
            limit: error.limit == null ? null : String(error.limit),
            actual: error.actual == null ? null : String(error.actual),
            details: error.details ?? null,
        });
    }
}

export function emptyV2CreateRecord(): Record<string, unknown> {
    return Object.create(null) as Record<string, unknown>;
}

export function isV2CreateRecord(value: unknown): value is Record<string, unknown> {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return false;
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
}

export function hasOwnV2CreateKey(value: Record<string, unknown>, key: string): boolean {
    return Object.prototype.hasOwnProperty.call(value, key);
}

export function ownV2CreateValue(value: Record<string, unknown>, key: string): unknown {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (descriptor === undefined) return undefined;
    if (!('value' in descriptor)) {
        throw invalidV2CreateRequestError(
            `NativeEditorBridge: accessor ${key} is not allowed for v2 create`
        );
    }
    return descriptor.value;
}

export function requireKnownV2CreateKeys(
    value: unknown,
    allowed: ReadonlySet<string>,
    label: string
): asserts value is Record<string, unknown> {
    if (
        !isV2CreateRecord(value) ||
        Reflect.ownKeys(value).some((key) => typeof key !== 'string' || !allowed.has(key))
    ) {
        throw invalidV2CreateRequestError(`NativeEditorBridge: invalid ${label} for v2 create`);
    }
}

export function normalizeV2CreateRecord(
    value: Record<string, unknown>,
    allowed: ReadonlySet<string>,
    label: string
): Record<string, unknown> {
    requireKnownV2CreateKeys(value, allowed, label);
    const normalized = emptyV2CreateRecord();
    for (const key of allowed) {
        if (!hasOwnV2CreateKey(value, key)) continue;
        const fieldValue = ownV2CreateValue(value, key);
        if (fieldValue === null) {
            throw invalidV2CreateRequestError(`NativeEditorBridge: invalid ${label} for v2 create`);
        }
        if (fieldValue !== undefined) normalized[key] = fieldValue;
    }
    return normalized;
}
