import { allocateEditorUpdateRevision } from '../EditorUpdateRevision';

describe('allocateEditorUpdateRevision', () => {
    it('allocates the maximum editor update revision once then reports exhaustion', () => {
        expect(allocateEditorUpdateRevision(0xffff_fffe)).toEqual({ revision: 0xffff_ffff });
        expect(allocateEditorUpdateRevision(0xffff_ffff)).toEqual({
            error: {
                domain: 'boundary',
                code: 'CONFIG_INVALID',
                message: 'NativeRichTextEditor: editor update revision counter exhausted',
                requestId: null,
                operationIndex: null,
                limit: '4294967295',
                actual: '4294967296',
                details: null,
            },
        });
    });
});
