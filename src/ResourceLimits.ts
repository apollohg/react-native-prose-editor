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

function resolveLimit(name: keyof ResolvedEditorResourceLimits, value?: number): number {
    const resolved = value ?? DEFAULT_EDITOR_RESOURCE_LIMITS[name];
    if (
        !Number.isSafeInteger(resolved) ||
        resolved <= 0 ||
        resolved > HARD_EDITOR_RESOURCE_LIMITS[name]
    ) {
        throw new NativeEditorBoundaryError(
            'INVALID_RESOURCE_LIMIT',
            `${name} must be a positive integer no greater than ${HARD_EDITOR_RESOURCE_LIMITS[name]}`,
            HARD_EDITOR_RESOURCE_LIMITS[name],
            resolved
        );
    }
    return resolved;
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
