import './helpers/NativeEditorBridgeV2Fixture';
import {
    HUGE_U64_DECIMAL,
    mockNativeModule,
    createHandle,
    catchThrown,
} from './helpers/NativeEditorBridgeV2Fixture';

import { NativeEditorV2BoundaryError, NativeEditorV2ErrorBase } from '../NativeEditorBoundaryError';

describe('NativeEditorBridge v2', () => {
    describe('request envelopes', () => {
        it('builds the exact input envelope with auto-assigned request ids', () => {
            const handle = createHandle();
            handle.bridge.applyInput({ baseDocumentRevision: '4', text: 'hi' });
            handle.bridge.applyInput({ baseDocumentRevision: '5', text: 'there' });
            const calls = mockNativeModule.editorV2ApplyInput.mock.calls;
            expect(calls[0][0]).toBe('1');
            expect(JSON.parse(calls[0][1])).toEqual({
                version: 1,
                requestId: '1',
                baseDocumentRevision: '4',
                text: 'hi',
            });
            expect(JSON.parse(calls[1][1])).toEqual({
                version: 1,
                requestId: '2',
                baseDocumentRevision: '5',
                text: 'there',
            });
        });

        it('accepts u64::MAX once then rejects request-ID exhaustion locally', () => {
            const handle = createHandle();
            const bridgeForTest = handle.bridge as unknown as { _nextRequestId: bigint };
            bridgeForTest._nextRequestId = BigInt(HUGE_U64_DECIMAL) - 1n;

            handle.bridge.undo();
            expect(JSON.parse(mockNativeModule.editorV2Undo.mock.calls[0][1])).toEqual({
                version: 1,
                requestId: HUGE_U64_DECIMAL,
            });

            const error = catchThrown(() => handle.bridge.redo());
            expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
            expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
            expect((error as NativeEditorV2ErrorBase).domain).toBe('boundary');
            expect(mockNativeModule.editorV2Redo).not.toHaveBeenCalled();
        });

        it('embeds a huge baseDocumentRevision as canonical decimal text without Number()', () => {
            const handle = createHandle();
            handle.bridge.applyInput({ baseDocumentRevision: HUGE_U64_DECIMAL, text: 'x' });
            const requestJson = mockNativeModule.editorV2ApplyInput.mock.calls[0][1];
            expect(requestJson).toBe(
                `{"version":1,"requestId":"1","baseDocumentRevision":"${HUGE_U64_DECIMAL}","text":"x"}`
            );
        });

        it.each(['01', '1.5', '-1', '', 'nope'])(
            'rejects a malformed baseDocumentRevision %p before any native call',
            (baseDocumentRevision) => {
                const handle = createHandle();
                const error = catchThrown(() =>
                    handle.bridge.applyInput({ baseDocumentRevision, text: 'x' })
                );
                expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
                expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
                expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
            }
        );

        it('rejects every numeric baseDocumentRevision before any native call', () => {
            const handle = createHandle();
            const error = catchThrown(() =>
                handle.bridge.applyInput({
                    baseDocumentRevision: Number.MAX_SAFE_INTEGER as never,
                    text: 'x',
                })
            );
            expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
            expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
        });

        it('builds the exact undo/redo envelope and treats changed:false as success', () => {
            const handle = createHandle();
            expect(handle.bridge.undo()).toBe(true);
            expect(handle.bridge.redo()).toBe(false);
            expect(JSON.parse(mockNativeModule.editorV2Undo.mock.calls[0][1])).toEqual({
                version: 1,
                requestId: '1',
            });
            expect(JSON.parse(mockNativeModule.editorV2Redo.mock.calls[0][1])).toEqual({
                version: 1,
                requestId: '2',
            });
        });

        it('builds the exact replace envelope and normalizes the commit revision', () => {
            const handle = createHandle();
            const commit = handle.bridge.replaceDocument({
                setJson: { type: 'doc', content: [] },
                history: 'resetAndClear',
            });
            expect(JSON.parse(mockNativeModule.editorV2ReplaceDocument.mock.calls[0][1])).toEqual({
                version: 1,
                requestId: '1',
                setJson: { type: 'doc', content: [] },
                history: 'resetAndClear',
            });
            expect(commit).toEqual({ changed: true, documentRevision: '6' });
        });
    });
});
