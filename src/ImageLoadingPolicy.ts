import { NativeEditorBoundaryError } from './NativeEditorBoundaryError';

export interface EditorImageLoadingPolicy {
    maxSourceBytes?: number;
    connectTimeoutMs?: number;
    readTimeoutMs?: number;
    requestTimeoutMs?: number;
    maxConcurrentRequests?: number;
    maxPendingRequests?: number;
    maxDecodeDimensionPx?: number;
}

export interface ResolvedEditorImageLoadingPolicy {
    maxSourceBytes: number;
    connectTimeoutMs: number;
    readTimeoutMs: number;
    requestTimeoutMs: number;
    maxConcurrentRequests: number;
    maxPendingRequests: number;
    maxDecodeDimensionPx: number;
}

export const DEFAULT_EDITOR_IMAGE_LOADING_POLICY: Readonly<ResolvedEditorImageLoadingPolicy> = {
    maxSourceBytes: 10 * 1024 * 1024,
    connectTimeoutMs: 10_000,
    readTimeoutMs: 20_000,
    requestTimeoutMs: 60_000,
    maxConcurrentRequests: 2,
    maxPendingRequests: 64,
    maxDecodeDimensionPx: 2_048,
};

export const HARD_EDITOR_IMAGE_LOADING_POLICY: Readonly<ResolvedEditorImageLoadingPolicy> = {
    maxSourceBytes: 64 * 1024 * 1024,
    connectTimeoutMs: 600_000,
    readTimeoutMs: 600_000,
    requestTimeoutMs: 600_000,
    maxConcurrentRequests: 16,
    maxPendingRequests: 512,
    maxDecodeDimensionPx: 8_192,
};

function resolveImagePolicyValue(
    name: keyof ResolvedEditorImageLoadingPolicy,
    value: number | undefined
): number {
    const resolved = value ?? DEFAULT_EDITOR_IMAGE_LOADING_POLICY[name];
    if (
        !Number.isSafeInteger(resolved) ||
        resolved <= 0 ||
        resolved > HARD_EDITOR_IMAGE_LOADING_POLICY[name]
    ) {
        throw new NativeEditorBoundaryError(
            'IMAGE_POLICY_INVALID',
            `${name} must be a positive integer no greater than ${HARD_EDITOR_IMAGE_LOADING_POLICY[name]}`,
            HARD_EDITOR_IMAGE_LOADING_POLICY[name],
            resolved
        );
    }
    return resolved;
}

export function resolveEditorImageLoadingPolicy(
    policy?: EditorImageLoadingPolicy
): ResolvedEditorImageLoadingPolicy {
    return {
        maxSourceBytes: resolveImagePolicyValue('maxSourceBytes', policy?.maxSourceBytes),
        connectTimeoutMs: resolveImagePolicyValue('connectTimeoutMs', policy?.connectTimeoutMs),
        readTimeoutMs: resolveImagePolicyValue('readTimeoutMs', policy?.readTimeoutMs),
        requestTimeoutMs: resolveImagePolicyValue('requestTimeoutMs', policy?.requestTimeoutMs),
        maxConcurrentRequests: resolveImagePolicyValue(
            'maxConcurrentRequests',
            policy?.maxConcurrentRequests
        ),
        maxPendingRequests: resolveImagePolicyValue('maxPendingRequests', policy?.maxPendingRequests),
        maxDecodeDimensionPx: resolveImagePolicyValue(
            'maxDecodeDimensionPx',
            policy?.maxDecodeDimensionPx
        ),
    };
}

export function serializeEditorImageLoadingPolicy(
    policy?: EditorImageLoadingPolicy
): string | undefined {
    return policy == null ? undefined : JSON.stringify(resolveEditorImageLoadingPolicy(policy));
}
