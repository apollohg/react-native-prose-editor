import './helpers/NativeEditorBridgeV2Fixture';
import {
    mockNativeModule,
    mockCollaborationTransportListeners,
    createHandle,
    flushMicrotasks,
} from './helpers/NativeEditorBridgeV2Fixture';

describe('NativeEditorBridge v2', () => {
    describe('collaboration subprotocol adapters', () => {
        it('keeps adapter callbacks in RN and reads current initialization data for every attempt', async () => {
            const handle = createHandle();
            let credential = 'first';
            const onOpen = jest.fn(async () => ({
                action: 'continue' as const,
                frames: [{ type: 'text' as const, data: `init:${credential}` }],
            }));

            handle.configureCollaborationTransport({
                url: 'wss://example.test/collaboration',
                connect: true,
                protocolAdapter: {
                    protocols: ['example-auth-v1'],
                    timeoutMillis: 5_000,
                    terminalCloseCodes: [4403, 4408],
                    onOpen,
                    onMessage: async () => ({ action: 'ready' as const }),
                },
            });

            const wireConfig = JSON.parse(
                mockNativeModule.editorV2CollaborationConfigureTransport.mock.calls[0][1]
            );
            expect(wireConfig).toEqual({
                url: 'wss://example.test/collaboration',
                connect: true,
                protocolAdapter: {
                    protocols: ['example-auth-v1'],
                    timeoutMillis: 5_000,
                    terminalCloseCodes: [4403, 4408],
                },
            });

            const emit = (event: unknown) => {
                for (const listener of mockCollaborationTransportListeners) listener(event);
            };
            emit({
                editorId: handle.editorId,
                eventSequence: '1',
                generation: '7',
                kind: 'protocolAdapter',
                attemptId: 'attempt-1',
                eventId: '1',
                phase: 'open',
                negotiatedProtocol: 'example-auth-v1',
            });
            await flushMicrotasks();

            credential = 'second';
            emit({
                editorId: handle.editorId,
                eventSequence: '2',
                generation: '8',
                kind: 'protocolAdapter',
                attemptId: 'attempt-2',
                eventId: '1',
                phase: 'open',
                negotiatedProtocol: 'example-auth-v1',
            });
            await flushMicrotasks();

            expect(onOpen).toHaveBeenCalledTimes(2);
            expect(
                mockNativeModule.editorV2CollaborationResolveProtocolAdapter
            ).toHaveBeenNthCalledWith(
                1,
                handle.editorId,
                'attempt-1',
                '1',
                '{"action":"continue","frames":[{"type":"text","data":"init:first"}]}'
            );
            expect(
                mockNativeModule.editorV2CollaborationResolveProtocolAdapter
            ).toHaveBeenNthCalledWith(
                2,
                handle.editorId,
                'attempt-2',
                '1',
                '{"action":"continue","frames":[{"type":"text","data":"init:second"}]}'
            );
        });

        it('delivers pre-open frames only to the adapter and serializes binary replies as base64', async () => {
            const handle = createHandle();
            const onMessage = jest.fn(async () => ({
                action: 'ready' as const,
                frames: [{ type: 'binary' as const, data: new Uint8Array([0, 1, 254, 255]) }],
            }));
            handle.configureCollaborationTransport({
                url: 'wss://example.test/collaboration',
                connect: true,
                protocolAdapter: {
                    protocols: ['challenge-v1'],
                    onOpen: async () => ({ action: 'continue' as const }),
                    onMessage,
                },
            });

            for (const listener of mockCollaborationTransportListeners) {
                listener({
                    editorId: handle.editorId,
                    eventSequence: '1',
                    generation: '7',
                    kind: 'protocolAdapter',
                    attemptId: 'attempt-1',
                    eventId: '2',
                    phase: 'message',
                    negotiatedProtocol: 'challenge-v1',
                    frame: { type: 'binary', data: 'AQID' },
                });
            }
            await flushMicrotasks();

            expect(onMessage).toHaveBeenCalledWith(
                expect.objectContaining({
                    attemptId: 'attempt-1',
                    generation: '7',
                }),
                { type: 'binary', data: new Uint8Array([1, 2, 3]) }
            );
            expect(
                mockNativeModule.editorV2CollaborationResolveProtocolAdapter
            ).toHaveBeenCalledWith(
                handle.editorId,
                'attempt-1',
                '2',
                '{"action":"ready","frames":[{"type":"binary","data":"AAH+/w=="}]}'
            );
        });
    });
});
