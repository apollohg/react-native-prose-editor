import {
    resolveDocumentDescriptor,
    type ResolvedDocumentSchema,
    type SchemaDefinition,
} from './schemas';
import { type EditorResourceLimits } from './ResourceLimits';
import {
    normalizeV2CreateRecord,
    V2_CREATE_POLICY_KEYS,
    ownV2CreateValue,
    V2_CREATE_MAX_U32,
    V2_CREATE_SNAPSHOT_METADATA_KEYS,
    isV2CreateRecord,
    invalidV2CreateRequestError,
    requireKnownV2CreateKeys,
    V2_CREATE_CONFIG_KEYS,
    V2_CREATE_LIMIT_KEYS,
    emptyV2CreateRecord,
    V2_CREATE_RESOURCE_LIMIT_KEYS,
    V2_CREATE_EDITING_LIMIT_KEYS,
    V2_CREATE_COLLABORATION_LIMIT_KEYS,
    V2_CREATE_INITIALIZATION_KEYS,
    V2_CREATE_ROOM_SNAPSHOT_KEYS,
    NativeEditorCreateConfigError,
    validateV2CreateLimits,
} from './NativeEditorCreateValidation';
import {
    invalidV2JsonValue,
    type V2JsonNormalizationTraversal,
    normalizeV2JsonValue,
    serializeV2CreateEnvelope,
} from './NativeEditorCreateJson';
import { type NativeEditorCreateConfig } from './NativeEditorTypes';
import { requireV2Bytes, invalidV2RequestError } from './NativeEditorResultNormalization';

export function normalizeV2CreatePolicy(value: Record<string, unknown>): Record<string, unknown> {
    const policy = normalizeV2CreateRecord(value, V2_CREATE_POLICY_KEYS, 'policy');
    const maxLength = ownV2CreateValue(policy, 'maxLength');
    if (
        maxLength !== undefined &&
        (typeof maxLength !== 'number' ||
            !Number.isSafeInteger(maxLength) ||
            maxLength < 0 ||
            maxLength > V2_CREATE_MAX_U32)
    ) {
        invalidV2JsonValue('policy.maxLength');
    }
    for (const key of ['readOnly', 'allowBase64Images']) {
        const value = ownV2CreateValue(policy, key);
        if (value !== undefined && typeof value !== 'boolean') {
            invalidV2JsonValue(`policy.${key}`);
        }
    }
    const inputFilter = ownV2CreateValue(policy, 'inputFilter');
    if (inputFilter !== undefined && typeof inputFilter !== 'string') {
        invalidV2JsonValue('policy.inputFilter');
    }
    return policy;
}

export function normalizeV2SnapshotMetadata(value: unknown): Record<string, unknown> {
    const metadata = normalizeV2CreateRecord(
        value as Record<string, unknown>,
        V2_CREATE_SNAPSHOT_METADATA_KEYS,
        'snapshot metadata'
    );
    const formatVersion = ownV2CreateValue(metadata, 'formatVersion');
    if (
        typeof formatVersion !== 'number' ||
        !Number.isSafeInteger(formatVersion) ||
        formatVersion < 0 ||
        formatVersion > V2_CREATE_MAX_U32
    ) {
        invalidV2JsonValue('snapshot metadata.formatVersion');
    }
    for (const key of ['documentId', 'lineageId', 'fragmentName', 'schemaFingerprint']) {
        if (typeof ownV2CreateValue(metadata, key) !== 'string') {
            invalidV2JsonValue(`snapshot metadata.${key}`);
        }
    }
    return metadata;
}

export function buildV2CreateRequestUnchecked(config: NativeEditorCreateConfig): {
    envelope: Record<string, unknown>;
    limits: NativeEditorCreateConfig['limits'];
    snapshotState: Uint8Array | null;
} {
    if (!isV2CreateRecord(config)) {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid v2 create config');
    }
    requireKnownV2CreateKeys(config, V2_CREATE_CONFIG_KEYS, 'config');
    const initializationValue = ownV2CreateValue(config, 'initialization');
    if (!isV2CreateRecord(initializationValue)) {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid v2 create config');
    }
    const jsonTraversal: V2JsonNormalizationTraversal = {
        seen: new WeakSet<object>(),
        work: 0,
    };

    const policyValue = ownV2CreateValue(config, 'policy');
    const policy =
        policyValue === undefined
            ? undefined
            : normalizeV2CreatePolicy(policyValue as Record<string, unknown>);

    const limitsValue = ownV2CreateValue(config, 'limits');
    let limits: Record<string, unknown> | undefined;
    if (limitsValue !== undefined) {
        requireKnownV2CreateKeys(limitsValue, V2_CREATE_LIMIT_KEYS, 'limits');
        limits = emptyV2CreateRecord();
        for (const [group, keys] of [
            ['resource', V2_CREATE_RESOURCE_LIMIT_KEYS],
            ['editing', V2_CREATE_EDITING_LIMIT_KEYS],
            ['collaboration', V2_CREATE_COLLABORATION_LIMIT_KEYS],
        ] as const) {
            const overrides = ownV2CreateValue(limitsValue, group);
            if (overrides !== undefined) {
                limits[group] = normalizeV2CreateRecord(
                    overrides as Record<string, unknown>,
                    keys,
                    `${group} limits`
                );
            }
        }
    }
    const envelope = emptyV2CreateRecord();
    const schema = ownV2CreateValue(config, 'schema');
    if (schema === null) {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid schema for v2 create');
    }
    if (schema !== undefined) {
        if (!isV2CreateRecord(schema)) {
            throw invalidV2CreateRequestError('NativeEditorBridge: invalid schema for v2 create');
        }
        envelope.schema = normalizeV2JsonValue(schema, 'schema', jsonTraversal);
    }
    const fragmentName = ownV2CreateValue(config, 'fragmentName');
    if (fragmentName !== undefined && typeof fragmentName !== 'string') {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid fragmentName for v2 create');
    }
    if (fragmentName !== undefined) envelope.fragmentName = fragmentName;

    let snapshotState: Uint8Array | null = null;
    const initialization = initializationValue;
    const initializationType = ownV2CreateValue(initialization, 'type');
    const initializationKeys =
        typeof initializationType === 'string'
            ? V2_CREATE_INITIALIZATION_KEYS[initializationType]
            : undefined;
    if (initializationKeys === undefined) {
        throw invalidV2CreateRequestError('NativeEditorBridge: unknown v2 initialization type');
    }
    requireKnownV2CreateKeys(initialization, initializationKeys, 'initialization');
    switch (initializationType) {
        case 'localEmpty': {
            const localEmpty = emptyV2CreateRecord();
            localEmpty.type = 'localEmpty';
            envelope.initialization = localEmpty;
            break;
        }
        case 'localJson': {
            const json = ownV2CreateValue(initialization, 'json');
            if (!isV2CreateRecord(json)) {
                throw invalidV2CreateRequestError(
                    'NativeEditorBridge: invalid localJson initialization for v2 create'
                );
            }
            const localJson = emptyV2CreateRecord();
            localJson.type = 'localJson';
            localJson.json = normalizeV2JsonValue(json, 'localJson initialization', jsonTraversal);
            envelope.initialization = localJson;
            break;
        }
        case 'localHtml': {
            const html = ownV2CreateValue(initialization, 'html');
            if (typeof html !== 'string') {
                throw invalidV2CreateRequestError(
                    'NativeEditorBridge: invalid localHtml initialization for v2 create'
                );
            }
            const localHtml = emptyV2CreateRecord();
            localHtml.type = 'localHtml';
            localHtml.html = html;
            envelope.initialization = localHtml;
            break;
        }
        case 'room': {
            const documentId = ownV2CreateValue(initialization, 'documentId');
            const lineageId = ownV2CreateValue(initialization, 'lineageId');
            if (typeof documentId !== 'string' || typeof lineageId !== 'string') {
                throw invalidV2CreateRequestError(
                    'NativeEditorBridge: invalid room initialization for v2 create'
                );
            }
            const room = emptyV2CreateRecord();
            room.type = 'room';
            room.documentId = documentId;
            room.lineageId = lineageId;
            const snapshot = ownV2CreateValue(initialization, 'snapshot');
            if (snapshot !== undefined) {
                requireKnownV2CreateKeys(snapshot, V2_CREATE_ROOM_SNAPSHOT_KEYS, 'room snapshot');
                const metadataValue = ownV2CreateValue(snapshot, 'metadata');
                const metadata = normalizeV2SnapshotMetadata(metadataValue);
                room.snapshot = metadata;
                snapshotState = requireV2Bytes(
                    ownV2CreateValue(snapshot, 'encodedState'),
                    'snapshot encodedState'
                );
            }
            envelope.initialization = room;
            break;
        }
        default:
            throw invalidV2CreateRequestError('NativeEditorBridge: unknown v2 initialization type');
    }
    if (policy !== undefined) envelope.policy = policy;
    if (limits !== undefined) envelope.limits = limits;
    return {
        envelope,
        limits: limits as NativeEditorCreateConfig['limits'],
        snapshotState,
    };
}

export function cloneAndFreezeDescriptorValue<T>(value: T): T {
    if (value == null || typeof value !== 'object') return value;

    type MutableDescriptorValue = Record<string, unknown>;
    const setOwnValue = (target: MutableDescriptorValue, key: string, child: unknown): void => {
        Object.defineProperty(target, key, {
            value: child,
            writable: true,
            enumerable: true,
            configurable: true,
        });
    };
    const cloneRoot: MutableDescriptorValue = (
        Array.isArray(value) ? [] : {}
    ) as MutableDescriptorValue;
    const clones = new Map<object, MutableDescriptorValue>([[value, cloneRoot]]);
    const pending: Array<{
        source: Record<string, unknown>;
        target: MutableDescriptorValue;
    }> = [{ source: value as Record<string, unknown>, target: cloneRoot }];
    const created = [cloneRoot];

    while (pending.length > 0) {
        const current = pending.pop();
        if (current === undefined) break;
        const keys = Array.isArray(current.source)
            ? Array.from({ length: (current.source as unknown[]).length }, (_, index) =>
                  String(index)
              )
            : Object.keys(current.source);
        for (const key of keys) {
            const child = current.source[key];
            if (child == null || typeof child !== 'object') {
                setOwnValue(current.target, key, child);
                continue;
            }
            const existing = clones.get(child);
            if (existing !== undefined) {
                setOwnValue(current.target, key, existing);
                continue;
            }
            const childClone: MutableDescriptorValue = (
                Array.isArray(child) ? [] : {}
            ) as MutableDescriptorValue;
            clones.set(child, childClone);
            created.push(childClone);
            setOwnValue(current.target, key, childClone);
            pending.push({
                source: child as Record<string, unknown>,
                target: childClone,
            });
        }
    }

    for (let index = created.length - 1; index >= 0; index -= 1) {
        Object.freeze(created[index]);
    }
    return cloneRoot as T;
}

export function cloneAndFreezeDocumentDescriptor(
    descriptor: ResolvedDocumentSchema
): ResolvedDocumentSchema {
    return Object.freeze({
        schema: cloneAndFreezeDescriptorValue(descriptor.schema),
        documentNodeName: descriptor.documentNodeName,
        emptyDocument: cloneAndFreezeDescriptorValue(descriptor.emptyDocument),
    });
}

export function buildV2CreateRequest(config: NativeEditorCreateConfig): {
    configJson: string;
    snapshotState: Uint8Array | null;
    documentDescriptor: ResolvedDocumentSchema;
} {
    let normalized: ReturnType<typeof buildV2CreateRequestUnchecked>;
    try {
        normalized = buildV2CreateRequestUnchecked(config);
    } catch (error) {
        const message =
            error instanceof NativeEditorCreateConfigError
                ? error.message
                : 'NativeEditorBridge: invalid v2 create config';
        throw invalidV2RequestError(message);
    }

    validateV2CreateLimits(normalized.limits);
    const documentDescriptor = cloneAndFreezeDocumentDescriptor(
        resolveDocumentDescriptor(
            normalized.envelope.schema as SchemaDefinition | undefined,
            normalized.limits?.resource as EditorResourceLimits | undefined
        )
    );
    try {
        const configJson = serializeV2CreateEnvelope(normalized.envelope);
        return { configJson, snapshotState: normalized.snapshotState, documentDescriptor };
    } catch {
        throw invalidV2RequestError('NativeEditorBridge: invalid v2 create config');
    }
}
