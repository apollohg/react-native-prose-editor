import { NativeEditorBoundaryError } from './NativeEditorBoundaryError';

/**
 * Bounds on the documents, schemas, and collaboration payloads the Rust core
 * will admit. Set them on `NativeEditorCreateConfig.limits.resource` when
 * creating a document handle, or on `RichTextViewer.resourceLimits`.
 *
 * Every field is optional; an omitted one uses
 * {@link DEFAULT_EDITOR_RESOURCE_LIMITS}. Each value must be a positive
 * integer no greater than the matching {@link HARD_EDITOR_RESOURCE_LIMITS}
 * ceiling — anything else throws `NativeEditorBoundaryError`
 * (`INVALID_RESOURCE_LIMIT`). Exceeding a limit at runtime raises
 * `INPUT_LIMIT_EXCEEDED`, `DOCUMENT_LIMIT_EXCEEDED`, or `SCHEMA_INVALID`.
 */
export interface EditorResourceLimits {
    /** Byte ceiling for a single config, document JSON, or HTML string crossing into the core. */
    maxInputBytes?: number;
    /** Maximum number of nodes in a document. */
    maxDocumentNodes?: number;
    /** Maximum node nesting depth in a document. */
    maxDocumentDepth?: number;
    /** Maximum number of node specs in a schema definition. */
    maxSchemaNodes?: number;
    /** Byte ceiling for the combined `content` expressions of a schema's nodes. */
    maxSchemaExpressionBytes?: number;
    /** Byte ceiling for one inbound collaboration message. */
    maxCollaborationMessageBytes?: number;
    /** Byte ceiling for an encoded Yjs state — imported room snapshots and exported ones. */
    maxEncodedStateBytes?: number;
}

/**
 * Bounds on editing work inside the engine, set on
 * `NativeEditorCreateConfig.limits.editing`. Omitted fields use the Rust
 * core's defaults; supplied values are validated against the same hard
 * ceilings the core enforces.
 */
export interface EditorEditingLimits {
    /** Maximum operations one compiled transaction may carry. Core default 256. */
    maxOperationsPerTransaction?: number;
    /** Maximum undoable groups retained in history. Core default 500. */
    maxUndoGroups?: number;
    /** Maximum document units the undo stack may retain across all groups. Core default 1,000,000. */
    maxUndoRetainedUnits?: number;
    /** Byte ceiling for derived output (HTML, JSON, render state) the engine produces. Core default 32 MiB. */
    maxDerivedOutputBytes?: number;
}

/**
 * Bounds on the native collaboration transport, set on
 * `NativeEditorCreateConfig.limits.collaboration`. Omitted fields use the
 * Rust core's defaults; supplied values are validated against the same hard
 * ceilings the core enforces.
 */
export interface EditorCollaborationLimits {
    /** Maximum protocol frames decoded from one transport message. Core default 64. */
    maxFramesPerMessage?: number;
    /** Byte ceiling for a single protocol frame. Core default 10 MiB. */
    maxFrameBytes?: number;
    /** Byte ceiling for all reply frames produced for one inbound message. Core default 10 MiB. */
    maxAggregateResponseBytes?: number;
    /** Maximum tracked awareness peers. Core default 1,024. */
    maxAwarenessPeers?: number;
    /** Byte ceiling for one peer's awareness state. Core default 64 KiB. */
    maxAwarenessPeerBytes?: number;
    /** Byte ceiling for all retained awareness state. Core default 10 MiB. */
    maxAwarenessBytes?: number;
    /** Maximum messages queued for send while disconnected. Core default 256. */
    maxPendingOutboxMessages?: number;
    /** Byte ceiling for the outbound queue. Core default 10 MiB. */
    maxPendingOutboxBytes?: number;
    /** Byte ceiling for remote updates buffered while awaiting their missing dependencies. Core default 10 MiB. */
    maxPendingDependencyUpdateBytes?: number;
    /** Work ceiling for those buffered dependency updates. Core default 1,000,000. */
    maxPendingDependencyUpdateWork?: number;
}

/** {@link EditorResourceLimits} with every default filled in. */
export interface ResolvedEditorResourceLimits {
    maxInputBytes: number;
    maxDocumentNodes: number;
    maxDocumentDepth: number;
    maxSchemaNodes: number;
    maxSchemaExpressionBytes: number;
    maxCollaborationMessageBytes: number;
    maxEncodedStateBytes: number;
}

/** The value used for each {@link EditorResourceLimits} field left unset. */
export const DEFAULT_EDITOR_RESOURCE_LIMITS: Readonly<ResolvedEditorResourceLimits> = {
    maxInputBytes: 20 * 1024 * 1024,
    maxDocumentNodes: 100_000,
    maxDocumentDepth: 256,
    maxSchemaNodes: 1_024,
    maxSchemaExpressionBytes: 64 * 1024,
    maxCollaborationMessageBytes: 10 * 1024 * 1024,
    maxEncodedStateBytes: 50 * 1024 * 1024,
};

/** The ceiling each {@link EditorResourceLimits} field may not exceed. */
export const HARD_EDITOR_RESOURCE_LIMITS: Readonly<ResolvedEditorResourceLimits> = {
    maxInputBytes: 64 * 1024 * 1024,
    maxDocumentNodes: 1_000_000,
    maxDocumentDepth: 1_024,
    maxSchemaNodes: 10_000,
    maxSchemaExpressionBytes: 1024 * 1024,
    maxCollaborationMessageBytes: 64 * 1024 * 1024,
    maxEncodedStateBytes: 256 * 1024 * 1024,
};

const HARD_EDITOR_EDITING_LIMITS: Readonly<Required<EditorEditingLimits>> = {
    maxOperationsPerTransaction: 4_096,
    maxUndoGroups: 2_000,
    maxUndoRetainedUnits: 8_000_000,
    maxDerivedOutputBytes: 128 * 1024 * 1024,
};

const HARD_EDITOR_COLLABORATION_LIMITS: Readonly<Required<EditorCollaborationLimits>> = {
    maxFramesPerMessage: 1_024,
    maxFrameBytes: 64 * 1024 * 1024,
    maxAggregateResponseBytes: 64 * 1024 * 1024,
    maxAwarenessPeers: 10_000,
    maxAwarenessPeerBytes: 1024 * 1024,
    maxAwarenessBytes: 64 * 1024 * 1024,
    maxPendingOutboxMessages: 4_096,
    maxPendingOutboxBytes: 64 * 1024 * 1024,
    maxPendingDependencyUpdateBytes: 64 * 1024 * 1024,
    maxPendingDependencyUpdateWork: 8_000_000,
};

function validateLimit(name: string, value: number, ceiling: number): void {
    if (!Number.isSafeInteger(value) || value <= 0 || value > ceiling) {
        throw new NativeEditorBoundaryError(
            'INVALID_RESOURCE_LIMIT',
            `${name} must be a positive integer no greater than ${ceiling}`,
            ceiling,
            value
        );
    }
}

function resolveLimit(name: keyof ResolvedEditorResourceLimits, value?: number): number {
    const resolved = value ?? DEFAULT_EDITOR_RESOURCE_LIMITS[name];
    validateLimit(name, resolved, HARD_EDITOR_RESOURCE_LIMITS[name]);
    return resolved;
}

function validateOverrides<T extends object>(
    limits: T | undefined,
    ceilings: Readonly<Required<T>>
): void {
    if (limits === undefined) return;
    for (const name of Object.keys(ceilings) as Array<keyof T>) {
        const value = limits[name];
        if (value !== undefined) {
            validateLimit(String(name), value as number, ceilings[name] as number);
        }
    }
}

export function validateEditorCreateLimits(limits?: {
    resource?: EditorResourceLimits;
    editing?: EditorEditingLimits;
    collaboration?: EditorCollaborationLimits;
}): void {
    validateOverrides(limits?.resource, HARD_EDITOR_RESOURCE_LIMITS);
    validateOverrides(limits?.editing, HARD_EDITOR_EDITING_LIMITS);
    validateOverrides(limits?.collaboration, HARD_EDITOR_COLLABORATION_LIMITS);
}

/**
 * Fill in every unset {@link EditorResourceLimits} field from
 * {@link DEFAULT_EDITOR_RESOURCE_LIMITS}, validating each value against
 * {@link HARD_EDITOR_RESOURCE_LIMITS}.
 *
 * @throws NativeEditorBoundaryError `INVALID_RESOURCE_LIMIT` when a value is
 * not a positive integer within its ceiling.
 */
export function resolveEditorResourceLimits(
    limits: EditorResourceLimits = {}
): ResolvedEditorResourceLimits {
    return Object.fromEntries(
        (Object.keys(DEFAULT_EDITOR_RESOURCE_LIMITS) as Array<
            keyof ResolvedEditorResourceLimits
        >).map((name) => [name, resolveLimit(name, limits[name])])
    ) as unknown as ResolvedEditorResourceLimits;
}
