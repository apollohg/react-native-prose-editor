import './helpers/NativeEditorBridgeV2Fixture';
import { createHandle, expectNonRetryable } from './helpers/NativeEditorBridgeV2Fixture';

import {
    NativeEditorDocumentError,
    NativeEditorErrorBase,
    NativeEditorOperationError,
    type NativeEditorError,
} from '../NativeEditorBoundaryError';

describe('NativeEditorBridge v2', () => {
    describe('autonomous error events', () => {
        it('delivers exactly one typed error per emission', () => {
            const handle = createHandle();
            const received: NativeEditorErrorBase[] = [];
            handle.addErrorListener((error) => received.push(error));
            handle.bridge._emitAutonomousError({
                domain: 'operation',
                code: 'POSITION_INVALID',
                message: 'position invalid',
                requestId: '3',
            });
            expect(received).toHaveLength(1);
            expect(received[0]).toBeInstanceOf(NativeEditorOperationError);
            expect(received[0].code).toBe('POSITION_INVALID');
            expect(received[0].requestId).toBe('3');
        });

        it('accepts the frozen envelope form for autonomous errors', () => {
            const handle = createHandle();
            const received: NativeEditorErrorBase[] = [];
            handle.addErrorListener((error) => received.push(error));
            handle.bridge._emitAutonomousError({
                ok: false,
                error: { domain: 'document', code: 'DOCUMENT_INVALID', message: 'invalid' },
            });
            expect(received).toHaveLength(1);
            expect(received[0]).toBeInstanceOf(NativeEditorDocumentError);
        });

        it('reports a malformed autonomous error as a non-retryable contract violation', () => {
            const handle = createHandle();
            const received: NativeEditorErrorBase[] = [];
            handle.addErrorListener((error) => received.push(error));
            handle.bridge._emitAutonomousError({ code: 42 });
            expect(received).toHaveLength(1);
            expectNonRetryable(received[0], 'FFI_RESULT_INVALID');
        });

        it('stops delivery after unsubscribe and after destroy', () => {
            const handle = createHandle();
            const received: NativeEditorErrorBase[] = [];
            const unsubscribe = handle.addErrorListener((error) => received.push(error));
            const emission: NativeEditorError = {
                domain: 'operation',
                code: 'OPERATION_INVALID',
                message: 'operation invalid',
                requestId: null,
                operationIndex: null,
                limit: null,
                actual: null,
                details: null,
            };
            handle.bridge._emitAutonomousError(emission);
            unsubscribe();
            handle.bridge._emitAutonomousError(emission);
            expect(received).toHaveLength(1);
            handle.addErrorListener((error) => received.push(error));
            handle.destroy();
            handle.bridge._emitAutonomousError(emission);
            expect(received).toHaveLength(1);
        });
    });
});
