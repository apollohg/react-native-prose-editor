import { NativeEditorBoundaryError } from './NativeEditorBoundaryError';

export interface EditorResourceLimits {
    maxInputBytes?: number;
    maxDocumentNodes?: number;
    maxDocumentDepth?: number;
    maxSchemaNodes?: number;
    maxSchemaExpressionBytes?: number;
    maxCollaborationMessageBytes?: number;
    maxEncodedStateBytes?: number;
}

export interface EditorEditingLimits {
    maxOperationsPerTransaction?: number;
    maxUndoGroups?: number;
    maxUndoRetainedUnits?: number;
    maxDerivedOutputBytes?: number;
}

export interface EditorCollaborationLimits {
    maxFramesPerMessage?: number;
    maxFrameBytes?: number;
    maxAggregateResponseBytes?: number;
    maxAwarenessPeers?: number;
    maxAwarenessPeerBytes?: number;
    maxAwarenessBytes?: number;
    maxPendingOutboxMessages?: number;
    maxPendingOutboxBytes?: number;
    maxPendingDependencyUpdateBytes?: number;
    maxPendingDependencyUpdateWork?: number;
}

export interface ResolvedEditorResourceLimits {
    maxInputBytes: number;
    maxDocumentNodes: number;
    maxDocumentDepth: number;
    maxSchemaNodes: number;
    maxSchemaExpressionBytes: number;
    maxCollaborationMessageBytes: number;
    maxEncodedStateBytes: number;
}

export const DEFAULT_EDITOR_RESOURCE_LIMITS: Readonly<ResolvedEditorResourceLimits> = {
    maxInputBytes: 20 * 1024 * 1024,
    maxDocumentNodes: 100_000,
    maxDocumentDepth: 256,
    maxSchemaNodes: 1_024,
    maxSchemaExpressionBytes: 64 * 1024,
    maxCollaborationMessageBytes: 10 * 1024 * 1024,
    maxEncodedStateBytes: 50 * 1024 * 1024,
};

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

export function resolveEditorResourceLimits(
    limits: EditorResourceLimits = {}
): ResolvedEditorResourceLimits {
    return Object.fromEntries(
        (Object.keys(DEFAULT_EDITOR_RESOURCE_LIMITS) as Array<
            keyof ResolvedEditorResourceLimits
        >).map((name) => [name, resolveLimit(name, limits[name])])
    ) as unknown as ResolvedEditorResourceLimits;
}
