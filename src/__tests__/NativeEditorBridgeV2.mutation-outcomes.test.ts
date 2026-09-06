import './helpers/NativeEditorBridgeV2Fixture';
import {
    okRecord,
    mockNativeModule,
    createHandle,
    expectNonRetryable,
    catchRejectedNativeRecord,
} from './helpers/NativeEditorBridgeV2Fixture';

describe('NativeEditorBridge v2', () => {
    describe('mutation outcomes', () => {
        it('normalizes the notApplicable outcome', () => {
            const handle = createHandle();
            expect(
                handle.bridge.setSelection({
                    baseDocumentRevision: '4',
                    selection: {
                        type: 'text',
                        anchor: { offset: 0, kind: 'scalar' },
                        head: { offset: 0, kind: 'scalar' },
                    },
                })
            ).toEqual({ type: 'notApplicable' });
        });

        it('normalizes the replacement outcome', () => {
            const handle = createHandle();
            expect(
                handle.bridge.applyLocalApi({
                    baseDocumentRevision: '4',
                    setHtml: '<p>x</p>',
                    history: 'undoableBoundary',
                })
            ).toEqual({ type: 'replacement', changed: true, documentRevision: '9' });
        });

        it('rejects an unknown outcome type', () => {
            const handle = createHandle();
            mockNativeModule.editorV2ApplyCommand.mockReturnValueOnce(
                okRecord(JSON.stringify({ type: 'surprise' }))
            );
            expectNonRetryable(
                catchRejectedNativeRecord(() =>
                    handle.bridge.applyCommand({ baseDocumentRevision: '4', command: {} })
                ),
                'FFI_RESULT_INVALID'
            );
        });
    });
});
