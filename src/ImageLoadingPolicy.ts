import { NativeEditorBoundaryError } from './NativeEditorBoundaryError';

/**
 * Bounds on the native image pipeline that fetches and decodes remote and
 * data-URL images. Set it on `RichTextEditor.imageLoadingPolicy` or
 * `RichTextViewer.imageLoadingPolicy`.
 *
 * Every field is optional; an omitted one uses
 * {@link DEFAULT_EDITOR_IMAGE_LOADING_POLICY}. Each value must be a positive
 * integer no greater than the matching
 * {@link HARD_EDITOR_IMAGE_LOADING_POLICY} ceiling — anything else throws
 * `NativeEditorBoundaryError` (`IMAGE_POLICY_INVALID`).
 */
export interface EditorImageLoadingPolicy {
    /** Byte ceiling for one image source, applied to remote responses and decoded data-URL payloads alike. */
    maxSourceBytes?: number;
    /** Socket connect timeout for a remote image request, in milliseconds. */
    connectTimeoutMs?: number;
    /** Socket read timeout for a remote image request, in milliseconds. */
    readTimeoutMs?: number;
    /** Overall deadline for one image request, in milliseconds. */
    requestTimeoutMs?: number;
    /** Image requests allowed in flight at once. Further requests queue. */
    maxConcurrentRequests?: number;
    /** Queued requests allowed while the in-flight slots are full. Requests beyond this are dropped. */
    maxPendingRequests?: number;
    /** Ceiling on decoded image width and height, in pixels. Larger images are downsampled. */
    maxDecodeDimensionPx?: number;
    /** Decoded pixel bytes this editor or viewer may retain at once. */
    maxDecodedBytes?: number;
}

/** {@link EditorImageLoadingPolicy} with every default filled in. */
export interface ResolvedEditorImageLoadingPolicy {
    maxSourceBytes: number;
    connectTimeoutMs: number;
    readTimeoutMs: number;
    requestTimeoutMs: number;
    maxConcurrentRequests: number;
    maxPendingRequests: number;
    maxDecodeDimensionPx: number;
    maxDecodedBytes: number;
}

/** The value used for each {@link EditorImageLoadingPolicy} field left unset. */
export const DEFAULT_EDITOR_IMAGE_LOADING_POLICY: Readonly<ResolvedEditorImageLoadingPolicy> = {
    maxSourceBytes: 10 * 1024 * 1024,
    connectTimeoutMs: 10_000,
    readTimeoutMs: 20_000,
    requestTimeoutMs: 60_000,
    maxConcurrentRequests: 2,
    maxPendingRequests: 64,
    maxDecodeDimensionPx: 2_048,
    maxDecodedBytes: 32 * 1024 * 1024,
};

/** The ceiling each {@link EditorImageLoadingPolicy} field may not exceed. */
export const HARD_EDITOR_IMAGE_LOADING_POLICY: Readonly<ResolvedEditorImageLoadingPolicy> = {
    maxSourceBytes: 64 * 1024 * 1024,
    connectTimeoutMs: 600_000,
    readTimeoutMs: 600_000,
    requestTimeoutMs: 600_000,
    maxConcurrentRequests: 16,
    maxPendingRequests: 512,
    maxDecodeDimensionPx: 8_192,
    maxDecodedBytes: 256 * 1024 * 1024,
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

/**
 * Fill in every unset {@link EditorImageLoadingPolicy} field from
 * {@link DEFAULT_EDITOR_IMAGE_LOADING_POLICY}, validating each value against
 * {@link HARD_EDITOR_IMAGE_LOADING_POLICY}.
 *
 * @throws NativeEditorBoundaryError `IMAGE_POLICY_INVALID` when a value is
 * not a positive integer within its ceiling.
 */
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
        maxPendingRequests: resolveImagePolicyValue(
            'maxPendingRequests',
            policy?.maxPendingRequests
        ),
        maxDecodeDimensionPx: resolveImagePolicyValue(
            'maxDecodeDimensionPx',
            policy?.maxDecodeDimensionPx
        ),
        maxDecodedBytes: resolveImagePolicyValue('maxDecodedBytes', policy?.maxDecodedBytes),
    };
}

export function serializeEditorImageLoadingPolicy(
    policy?: EditorImageLoadingPolicy
): string | undefined {
    return policy == null ? undefined : JSON.stringify(resolveEditorImageLoadingPolicy(policy));
}
