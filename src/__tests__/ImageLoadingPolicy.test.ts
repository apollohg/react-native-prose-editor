import {
    DEFAULT_EDITOR_IMAGE_LOADING_POLICY,
    HARD_EDITOR_IMAGE_LOADING_POLICY,
    resolveEditorImageLoadingPolicy,
} from '../ImageLoadingPolicy';

describe('EditorImageLoadingPolicy', () => {
    it('defaults decoded image ownership to 32 MiB', () => {
        expect(DEFAULT_EDITOR_IMAGE_LOADING_POLICY.maxDecodedBytes).toBe(32 * 1024 * 1024);
        expect(resolveEditorImageLoadingPolicy().maxDecodedBytes).toBe(32 * 1024 * 1024);
    });

    it('accepts decoded image budgets through 256 MiB', () => {
        expect(HARD_EDITOR_IMAGE_LOADING_POLICY.maxDecodedBytes).toBe(256 * 1024 * 1024);
        expect(
            resolveEditorImageLoadingPolicy({ maxDecodedBytes: 256 * 1024 * 1024 }).maxDecodedBytes
        ).toBe(256 * 1024 * 1024);
    });

    it.each([0, -1, 1.5, Number.MAX_SAFE_INTEGER, Number.POSITIVE_INFINITY])(
        'rejects invalid decoded image budget %p',
        (maxDecodedBytes) => {
            expect(() => resolveEditorImageLoadingPolicy({ maxDecodedBytes })).toThrow(
                /maxDecodedBytes/
            );
        }
    );
});
