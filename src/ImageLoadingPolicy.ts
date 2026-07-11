export interface EditorImageLoadingPolicy {
    maxSourceBytes?: number;
    connectTimeoutMs?: number;
    readTimeoutMs?: number;
    maxConcurrentRequests?: number;
    maxPendingRequests?: number;
    maxDecodeDimensionPx?: number;
}

export interface ResolvedEditorImageLoadingPolicy {
    maxSourceBytes: number;
    connectTimeoutMs: number;
    readTimeoutMs: number;
    maxConcurrentRequests: number;
    maxPendingRequests: number;
    maxDecodeDimensionPx: number;
}

export const DEFAULT_EDITOR_IMAGE_LOADING_POLICY: Readonly<ResolvedEditorImageLoadingPolicy> = {
    maxSourceBytes: 10 * 1024 * 1024,
    connectTimeoutMs: 10_000,
    readTimeoutMs: 20_000,
    maxConcurrentRequests: 2,
    maxPendingRequests: 64,
    maxDecodeDimensionPx: 2_048,
};

function positiveIntegerOrDefault(value: number | undefined, fallback: number): number {
    return typeof value === 'number' && Number.isSafeInteger(value) && value > 0
        ? value
        : fallback;
}

export function resolveEditorImageLoadingPolicy(
    policy?: EditorImageLoadingPolicy
): ResolvedEditorImageLoadingPolicy {
    return {
        maxSourceBytes: positiveIntegerOrDefault(
            policy?.maxSourceBytes,
            DEFAULT_EDITOR_IMAGE_LOADING_POLICY.maxSourceBytes
        ),
        connectTimeoutMs: positiveIntegerOrDefault(
            policy?.connectTimeoutMs,
            DEFAULT_EDITOR_IMAGE_LOADING_POLICY.connectTimeoutMs
        ),
        readTimeoutMs: positiveIntegerOrDefault(
            policy?.readTimeoutMs,
            DEFAULT_EDITOR_IMAGE_LOADING_POLICY.readTimeoutMs
        ),
        maxConcurrentRequests: positiveIntegerOrDefault(
            policy?.maxConcurrentRequests,
            DEFAULT_EDITOR_IMAGE_LOADING_POLICY.maxConcurrentRequests
        ),
        maxPendingRequests: positiveIntegerOrDefault(
            policy?.maxPendingRequests,
            DEFAULT_EDITOR_IMAGE_LOADING_POLICY.maxPendingRequests
        ),
        maxDecodeDimensionPx: positiveIntegerOrDefault(
            policy?.maxDecodeDimensionPx,
            DEFAULT_EDITOR_IMAGE_LOADING_POLICY.maxDecodeDimensionPx
        ),
    };
}

export function serializeEditorImageLoadingPolicy(
    policy?: EditorImageLoadingPolicy
): string | undefined {
    return policy == null ? undefined : JSON.stringify(resolveEditorImageLoadingPolicy(policy));
}
