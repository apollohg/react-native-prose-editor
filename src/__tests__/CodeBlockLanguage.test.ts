import './helpers/NativeEditorBridgeV2Fixture';
import { validRenderElement } from '../NativeEditorRenderNormalization';

it('accepts optional code language metadata and rejects malformed values', () => {
    expect(
        validRenderElement({
            type: 'blockStart',
            nodeType: 'codeBlock',
            depth: 0,
            language: 'rust',
        })
    ).toBe(true);
    expect(
        validRenderElement({ type: 'blockStart', nodeType: 'codeBlock', depth: 0, language: 42 })
    ).toBe(false);
    expect(validRenderElement({ type: 'blockStart', nodeType: 'paragraph', depth: 0 })).toBe(true);
});
