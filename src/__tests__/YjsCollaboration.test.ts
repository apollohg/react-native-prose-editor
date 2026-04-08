const INITIAL_DOC = {
    type: 'doc',
    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hello' }] }],
};

const REMOTE_DOC = {
    type: 'doc',
    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'remote' }] }],
};

const LOCAL_DOC = {
    type: 'doc',
    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'local' }] }],
};

const ALT_INITIAL_DOC = {
    type: 'doc',
    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'alt' }] }],
};

const CUSTOM_COLLABORATION_SCHEMA = {
    nodes: [
        { name: 'doc', content: 'block+', role: 'doc' },
        { name: 'paragraph', content: 'inline*', group: 'block', role: 'textBlock' },
        {
            name: 'calloutDivider',
            content: '',
            group: 'block',
            role: 'block',
            isVoid: true,
        },
        { name: 'text', content: '', group: 'inline', role: 'text' },
    ],
    marks: [],
};

const TITLE_FIRST_SCHEMA = {
    nodes: [
        { name: 'doc', content: 'title block*', role: 'doc' },
        { name: 'title', content: 'inline*', group: 'block', role: 'textBlock' },
        { name: 'paragraph', content: 'inline*', group: 'block', role: 'textBlock' },
        { name: 'text', content: '', group: 'inline', role: 'text' },
    ],
    marks: [],
};

const TITLE_EMPTY_DOC = {
    type: 'doc',
    content: [{ type: 'title' }],
};

const REMOTE_PEERS = [
    {
        clientId: 2,
        isLocal: false,
        state: {
            user: { userId: '2', name: 'Bob', color: '#00f' },
            selection: { anchor: 4, head: 9 },
            focused: true,
        },
    },
];

const mockNativeModule = {
    collaborationSessionCreate: jest.fn(() => 1),
    collaborationSessionDestroy: jest.fn(),
    collaborationSessionGetDocumentJson: jest.fn(() => JSON.stringify(INITIAL_DOC)),
    collaborationSessionGetEncodedState: jest.fn(() => JSON.stringify([9, 8, 7])),
    collaborationSessionGetPeersJson: jest.fn(() => '[]'),
    collaborationSessionStart: jest.fn(() =>
        JSON.stringify({
            messages: [
                [0, 0, 1],
                [1, 2, 3],
            ],
            documentChanged: false,
            peersChanged: false,
        })
    ),
    collaborationSessionApplyLocalDocumentJson: jest.fn(() =>
        JSON.stringify({
            messages: [[0, 2, 8, 8, 8]],
            documentChanged: true,
            documentJson: LOCAL_DOC,
            peersChanged: false,
        })
    ),
    collaborationSessionApplyEncodedState: jest.fn(() =>
        JSON.stringify({
            messages: [],
            documentChanged: true,
            documentJson: REMOTE_DOC,
            peersChanged: false,
        })
    ),
    collaborationSessionReplaceEncodedState: jest.fn(() =>
        JSON.stringify({
            messages: [[0, 2, 5, 5, 5]],
            documentChanged: true,
            documentJson: REMOTE_DOC,
            peersChanged: false,
        })
    ),
    collaborationSessionHandleMessage: jest.fn(() =>
        JSON.stringify({
            messages: [[4, 5, 6]],
            documentChanged: true,
            documentJson: REMOTE_DOC,
            peersChanged: true,
            peers: REMOTE_PEERS,
        })
    ),
    collaborationSessionSetLocalAwareness: jest.fn(() =>
        JSON.stringify({
            messages: [[1, 6, 6, 6]],
            documentChanged: false,
            peersChanged: false,
        })
    ),
    collaborationSessionClearLocalAwareness: jest.fn(() =>
        JSON.stringify({
            messages: [[7, 7, 7]],
            documentChanged: false,
            peersChanged: false,
        })
    ),
};

jest.mock('expo-modules-core', () => ({
    requireNativeModule: () => mockNativeModule,
}));

import React from 'react';
import { render, act } from '@testing-library/react-native';
import { createYjsCollaborationController, useYjsCollaboration } from '../YjsCollaboration';
import type { DocumentJSON } from '../NativeEditorBridge';
import { _resetNativeModuleCache } from '../NativeEditorBridge';

class MockWebSocket {
    static CONNECTING = 0;
    static OPEN = 1;
    static CLOSING = 2;
    static CLOSED = 3;

    readyState = MockWebSocket.CONNECTING;
    binaryType?: string;
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: unknown }) => void) | null = null;
    onerror: (() => void) | null = null;
    onclose: (() => void) | null = null;
    send = jest.fn();
    close = jest.fn(() => {
        this.readyState = MockWebSocket.CLOSED;
        this.onclose?.();
    });

    open(): void {
        this.readyState = MockWebSocket.OPEN;
        this.onopen?.();
    }

    receive(bytes: number[]): void {
        this.onmessage?.({ data: Uint8Array.from(bytes).buffer });
    }
}

describe('YjsCollaboration', () => {
    const OriginalWebSocket = global.WebSocket;

    beforeEach(() => {
        jest.useFakeTimers();
        _resetNativeModuleCache();
        for (const key of Object.keys(mockNativeModule)) {
            (mockNativeModule as Record<string, jest.Mock>)[key].mockClear();
        }
        mockNativeModule.collaborationSessionCreate.mockReturnValue(1);
        mockNativeModule.collaborationSessionGetDocumentJson.mockReturnValue(
            JSON.stringify(INITIAL_DOC)
        );
        mockNativeModule.collaborationSessionGetEncodedState.mockReturnValue(
            JSON.stringify([9, 8, 7])
        );
        mockNativeModule.collaborationSessionGetPeersJson.mockReturnValue('[]');
        global.WebSocket = MockWebSocket as unknown as typeof WebSocket;
    });

    afterAll(() => {
        global.WebSocket = OriginalWebSocket;
    });

    it('starts sync, applies remote updates, and publishes local edits', () => {
        const sockets: MockWebSocket[] = [];
        const peersSpy = jest.fn();
        const states: string[] = [];
        const controller = createYjsCollaborationController({
            documentId: 'doc-1',
            connect: false,
            createWebSocket: () => {
                const socket = new MockWebSocket();
                sockets.push(socket);
                return socket as unknown as WebSocket;
            },
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
            onPeersChange: peersSpy,
            onStateChange: (state) => {
                states.push(state.status);
            },
        });

        controller.connect();
        expect(controller.state.status).toBe('connecting');
        expect(sockets).toHaveLength(1);

        sockets[0].open();
        expect(controller.state.status).toBe('connected');
        expect(controller.state.isConnected).toBe(true);
        expect(sockets[0].binaryType).toBe('arraybuffer');
        expect(sockets[0].send).toHaveBeenCalledTimes(2);

        const firstOutbound = sockets[0].send.mock.calls[0][0] as ArrayBuffer;
        expect(Array.from(new Uint8Array(firstOutbound))).toEqual([0, 0, 1]);
        const secondOutbound = sockets[0].send.mock.calls[1][0] as ArrayBuffer;
        expect(Array.from(new Uint8Array(secondOutbound))).toEqual([1, 2, 3]);

        sockets[0].receive([9, 9, 9]);
        expect(mockNativeModule.collaborationSessionHandleMessage).toHaveBeenCalledWith(
            1,
            JSON.stringify([9, 9, 9])
        );
        expect(controller.state.documentJson).toEqual(REMOTE_DOC);
        expect(controller.peers).toEqual(REMOTE_PEERS);
        expect(peersSpy).toHaveBeenCalledWith(REMOTE_PEERS);

        controller.handleLocalDocumentChange(LOCAL_DOC);
        expect(mockNativeModule.collaborationSessionApplyLocalDocumentJson).toHaveBeenCalledWith(
            1,
            JSON.stringify(LOCAL_DOC)
        );
        expect(controller.state.documentJson).toEqual(LOCAL_DOC);

        controller.handleSelectionChange({ type: 'text', anchor: 3, head: 5 });
        jest.advanceTimersByTime(40);
        expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenLastCalledWith(
            1,
            JSON.stringify({
                user: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
                focused: true,
                selection: { anchor: 3, head: 5 },
            })
        );

        controller.handleFocusChange(true);
        expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenLastCalledWith(
            1,
            JSON.stringify({
                user: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
                focused: true,
                selection: { anchor: 3, head: 5 },
            })
        );

        controller.disconnect();
        expect(mockNativeModule.collaborationSessionClearLocalAwareness).toHaveBeenCalledWith(1);
        expect(controller.state.status).toBe('disconnected');
        expect(controller.state.isConnected).toBe(false);
        expect(controller.peers).toEqual([]);
        expect(states[0]).toBe('connecting');
        expect(states).toContain('connected');
        expect(states[states.length - 1]).toBe('disconnected');

        controller.destroy();
        expect(mockNativeModule.collaborationSessionDestroy).toHaveBeenCalledWith(1);
    });

    it('derives remote selection overlays from awareness peers', () => {
        const sockets: MockWebSocket[] = [];
        let latest: ReturnType<typeof useYjsCollaboration> | null = null;

        function Harness() {
            latest = useYjsCollaboration({
                documentId: 'doc-2',
                connect: false,
                createWebSocket: () => {
                    const socket = new MockWebSocket();
                    sockets.push(socket);
                    return socket as unknown as WebSocket;
                },
                initialDocumentJson: INITIAL_DOC,
                localAwareness: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
            });
            return null;
        }

        render(React.createElement(Harness));

        act(() => {
            latest?.connect();
        });
        act(() => {
            sockets[0].open();
            sockets[0].receive([9, 9, 9]);
        });

        expect(latest?.editorBindings.remoteSelections).toEqual([
            {
                clientId: 2,
                anchor: 4,
                head: 9,
                color: '#00f',
                isFocused: true,
                name: 'Bob',
            },
        ]);
    });

    it('treats remote selections as focused unless awareness explicitly says otherwise', () => {
        const sockets: MockWebSocket[] = [];
        let latest: ReturnType<typeof useYjsCollaboration> | null = null;

        mockNativeModule.collaborationSessionHandleMessage.mockReturnValueOnce(
            JSON.stringify({
                messages: [],
                documentChanged: false,
                peersChanged: true,
                peers: [
                    {
                        clientId: 2,
                        isLocal: false,
                        state: {
                            user: { userId: '2', name: 'Bob', color: '#00f' },
                            selection: { anchor: 6, head: 6 },
                        },
                    },
                ],
            })
        );

        function Harness() {
            latest = useYjsCollaboration({
                documentId: 'doc-2b',
                connect: false,
                createWebSocket: () => {
                    const socket = new MockWebSocket();
                    sockets.push(socket);
                    return socket as unknown as WebSocket;
                },
                initialDocumentJson: INITIAL_DOC,
                localAwareness: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
            });
            return null;
        }

        render(React.createElement(Harness));

        act(() => {
            latest?.connect();
        });
        act(() => {
            sockets[0].open();
            sockets[0].receive([9, 9, 9]);
        });

        expect(latest?.editorBindings.remoteSelections).toEqual([
            {
                clientId: 2,
                anchor: 6,
                head: 6,
                color: '#00f',
                isFocused: true,
                name: 'Bob',
            },
        ]);
    });

    it('coalesces rapid selection awareness updates', () => {
        const controller = createYjsCollaborationController({
            documentId: 'doc-3',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.handleSelectionChange({ type: 'text', anchor: 1, head: 2 });
        controller.handleSelectionChange({ type: 'text', anchor: 3, head: 5 });

        expect(mockNativeModule.collaborationSessionSetLocalAwareness).not.toHaveBeenCalled();

        jest.advanceTimersByTime(40);

        expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenCalledTimes(1);
        expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenLastCalledWith(
            1,
            JSON.stringify({
                user: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
                focused: true,
                selection: { anchor: 3, head: 5 },
            })
        );

        controller.destroy();
    });

    it('responds to y-websocket queryAwareness with a local awareness update', () => {
        const sockets: MockWebSocket[] = [];
        const controller = createYjsCollaborationController({
            documentId: 'doc-awareness',
            connect: false,
            createWebSocket: () => {
                const socket = new MockWebSocket();
                sockets.push(socket);
                return socket as unknown as WebSocket;
            },
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.connect();
        sockets[0].open();
        sockets[0].send.mockClear();
        mockNativeModule.collaborationSessionSetLocalAwareness.mockClear();

        sockets[0].receive([3]);

        expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenCalledWith(
            1,
            JSON.stringify({
                user: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
                focused: false,
            })
        );
        expect(mockNativeModule.collaborationSessionHandleMessage).not.toHaveBeenCalledWith(
            1,
            JSON.stringify([3])
        );
        expect(sockets[0].send).toHaveBeenCalledTimes(1);
        expect(Array.from(new Uint8Array(sockets[0].send.mock.calls[0][0] as ArrayBuffer))).toEqual(
            [1, 6, 6, 6]
        );

        controller.destroy();
    });

    it('flushes focus awareness changes immediately', () => {
        const controller = createYjsCollaborationController({
            documentId: 'doc-4',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.handleSelectionChange({ type: 'text', anchor: 1, head: 2 });
        controller.handleFocusChange(true);

        expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenCalledTimes(1);
        expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenLastCalledWith(
            1,
            JSON.stringify({
                user: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
                focused: true,
                selection: { anchor: 1, head: 2 },
            })
        );

        controller.destroy();
    });

    it('closes a connecting socket when disconnected before open', () => {
        const sockets: MockWebSocket[] = [];
        const controller = createYjsCollaborationController({
            documentId: 'doc-disconnect-connecting',
            connect: false,
            createWebSocket: () => {
                const socket = new MockWebSocket();
                sockets.push(socket);
                return socket as unknown as WebSocket;
            },
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.connect();
        expect(sockets).toHaveLength(1);

        controller.disconnect();

        expect(sockets[0].close).toHaveBeenCalledTimes(1);
        expect(mockNativeModule.collaborationSessionClearLocalAwareness).not.toHaveBeenCalled();
        expect(controller.state.status).toBe('disconnected');

        controller.destroy();
    });

    it('retries after an unexpected socket close using the default exponential backoff', () => {
        const sockets: MockWebSocket[] = [];
        const controller = createYjsCollaborationController({
            documentId: 'doc-retry-default',
            connect: false,
            createWebSocket: () => {
                const socket = new MockWebSocket();
                sockets.push(socket);
                return socket as unknown as WebSocket;
            },
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.connect();
        sockets[0].open();
        sockets[0].close.mockClear();

        sockets[0].close();

        expect(controller.state.status).toBe('disconnected');
        expect(controller.state.isConnected).toBe(false);
        expect(sockets).toHaveLength(1);

        jest.advanceTimersByTime(499);
        expect(sockets).toHaveLength(1);

        jest.advanceTimersByTime(1);
        expect(sockets).toHaveLength(2);
        expect(controller.state.status).toBe('connecting');

        controller.destroy();
    });

    it('uses a custom retry function and can stop retrying', () => {
        const sockets: MockWebSocket[] = [];
        const retryIntervalMs = jest
            .fn<number | null, [{ attempt: number; documentId: string; lastError?: Error }]>()
            .mockReturnValueOnce(250)
            .mockReturnValueOnce(null);
        const controller = createYjsCollaborationController({
            documentId: 'doc-retry-custom',
            connect: false,
            retryIntervalMs,
            createWebSocket: () => {
                const socket = new MockWebSocket();
                sockets.push(socket);
                return socket as unknown as WebSocket;
            },
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.connect();
        sockets[0].open();
        sockets[0].close();

        expect(retryIntervalMs).toHaveBeenCalledWith({
            attempt: 1,
            documentId: 'doc-retry-custom',
            lastError: undefined,
        });

        jest.advanceTimersByTime(250);
        expect(sockets).toHaveLength(2);

        sockets[1].open();
        sockets[1].close();

        expect(retryIntervalMs).toHaveBeenLastCalledWith({
            attempt: 1,
            documentId: 'doc-retry-custom',
            lastError: undefined,
        });

        jest.advanceTimersByTime(250);
        expect(sockets).toHaveLength(2);

        controller.destroy();
    });

    it('does not retry after a manual disconnect', () => {
        const sockets: MockWebSocket[] = [];
        const controller = createYjsCollaborationController({
            documentId: 'doc-retry-manual-stop',
            connect: false,
            retryIntervalMs: 1000,
            createWebSocket: () => {
                const socket = new MockWebSocket();
                sockets.push(socket);
                return socket as unknown as WebSocket;
            },
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.connect();
        sockets[0].open();
        sockets[0].close();

        controller.disconnect();

        jest.advanceTimersByTime(1000);
        expect(sockets).toHaveLength(1);

        controller.destroy();
    });

    it('supports disabling automatic retry', () => {
        const sockets: MockWebSocket[] = [];
        const controller = createYjsCollaborationController({
            documentId: 'doc-retry-disabled',
            connect: false,
            retryIntervalMs: false,
            createWebSocket: () => {
                const socket = new MockWebSocket();
                sockets.push(socket);
                return socket as unknown as WebSocket;
            },
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.connect();
        sockets[0].open();
        sockets[0].close();

        jest.advanceTimersByTime(5_000);
        expect(sockets).toHaveLength(1);

        controller.destroy();
    });

    it('surfaces websocket creation failures as error state', () => {
        const onError = jest.fn();
        const controller = createYjsCollaborationController({
            documentId: 'doc-5',
            connect: false,
            createWebSocket: () => {
                throw new Error('bad url');
            },
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
            onError,
        });

        controller.connect();

        expect(controller.state.status).toBe('error');
        expect(controller.state.isConnected).toBe(false);
        expect(controller.state.lastError?.message).toBe('bad url');
        expect(onError).toHaveBeenCalled();

        controller.destroy();
    });

    it('uses the native schema-valid empty document when no initial doc is provided', () => {
        mockNativeModule.collaborationSessionGetDocumentJson.mockReturnValue(
            JSON.stringify(TITLE_EMPTY_DOC)
        );

        const controller = createYjsCollaborationController({
            documentId: 'doc-6',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            schema: TITLE_FIRST_SCHEMA,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        expect(mockNativeModule.collaborationSessionCreate).toHaveBeenCalledWith(
            JSON.stringify({
                fragmentName: 'default',
                schema: TITLE_FIRST_SCHEMA,
                localAwareness: {
                    user: {
                        userId: '1',
                        name: 'Alice',
                        color: '#f00',
                    },
                    focused: false,
                },
            })
        );
        expect(controller.state.documentJson).toEqual(TITLE_EMPTY_DOC);

        controller.destroy();
    });

    it('uses a schema-aware empty document on the first hook render before native initialization completes', () => {
        const renderedDocs: DocumentJSON[] = [];
        let latest: ReturnType<typeof useYjsCollaboration> | null = null;
        mockNativeModule.collaborationSessionCreate.mockImplementation(() => {
            throw new Error('native init failed');
        });

        function Harness() {
            latest = useYjsCollaboration({
                documentId: 'doc-title-first',
                connect: false,
                createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
                schema: TITLE_FIRST_SCHEMA,
                localAwareness: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
            });
            renderedDocs.push(latest.state.documentJson);
            return null;
        }

        render(React.createElement(Harness));

        expect(renderedDocs[0]).toEqual(TITLE_EMPTY_DOC);
        expect(latest?.state.documentJson).toEqual(TITLE_EMPTY_DOC);
    });

    it('passes initial encoded state into the native collaboration session', () => {
        createYjsCollaborationController({
            documentId: 'doc-7',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            initialEncodedState: Uint8Array.from([4, 5, 6]),
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        expect(mockNativeModule.collaborationSessionCreate).toHaveBeenCalledWith(
            JSON.stringify({
                fragmentName: 'default',
                localAwareness: {
                    user: {
                        userId: '1',
                        name: 'Alice',
                        color: '#f00',
                    },
                    focused: false,
                },
            })
        );
        expect(mockNativeModule.collaborationSessionReplaceEncodedState).toHaveBeenCalledWith(
            1,
            JSON.stringify([4, 5, 6])
        );
    });

    it('passes a custom fragment name through to the native collaboration session', () => {
        createYjsCollaborationController({
            documentId: 'doc-fragment',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            fragmentName: 'prosemirror',
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        expect(mockNativeModule.collaborationSessionCreate).toHaveBeenCalledWith(
            JSON.stringify({
                fragmentName: 'prosemirror',
                localAwareness: {
                    user: {
                        userId: '1',
                        name: 'Alice',
                        color: '#f00',
                    },
                    focused: false,
                },
            })
        );
    });

    it('passes schema metadata through to the native collaboration session', () => {
        createYjsCollaborationController({
            documentId: 'doc-schema',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            schema: CUSTOM_COLLABORATION_SCHEMA,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        expect(mockNativeModule.collaborationSessionCreate).toHaveBeenCalledWith(
            JSON.stringify({
                fragmentName: 'default',
                schema: CUSTOM_COLLABORATION_SCHEMA,
                localAwareness: {
                    user: {
                        userId: '1',
                        name: 'Alice',
                        color: '#f00',
                    },
                    focused: false,
                },
            })
        );
    });

    it('uses initial document JSON as a local fallback without seeding the Yjs session', () => {
        mockNativeModule.collaborationSessionGetDocumentJson.mockReturnValue(
            JSON.stringify({
                type: 'doc',
                content: [{ type: 'paragraph' }],
            })
        );

        const controller = createYjsCollaborationController({
            documentId: 'doc-fallback',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        expect(mockNativeModule.collaborationSessionCreate).toHaveBeenCalledWith(
            JSON.stringify({
                fragmentName: 'default',
                localAwareness: {
                    user: {
                        userId: '1',
                        name: 'Alice',
                        color: '#f00',
                    },
                    focused: false,
                },
            })
        );
        expect(controller.state.documentJson).toEqual(INITIAL_DOC);

        controller.destroy();
    });

    it('surfaces invalid initial encoded state instead of silently ignoring it', () => {
        mockNativeModule.collaborationSessionReplaceEncodedState.mockReturnValueOnce(
            JSON.stringify({ error: 'invalid encoded state' })
        );

        expect(() =>
            createYjsCollaborationController({
                documentId: 'doc-invalid',
                connect: false,
                createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
                initialEncodedState: Uint8Array.from([4, 5, 6]),
                localAwareness: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
            })
        ).toThrow('NativeEditorBridge: invalid encoded state');
        expect(mockNativeModule.collaborationSessionDestroy).toHaveBeenCalledWith(1);
    });

    it('exposes the durable encoded collaboration state', () => {
        const controller = createYjsCollaborationController({
            documentId: 'doc-8',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        expect(controller.getEncodedState()).toEqual(Uint8Array.from([9, 8, 7]));
        expect(controller.getEncodedStateBase64()).toBe('CQgH');

        controller.destroy();
    });

    it('can merge an encoded collaboration state after creation', () => {
        const controller = createYjsCollaborationController({
            documentId: 'doc-9',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.applyEncodedState('AQID');

        expect(mockNativeModule.collaborationSessionApplyEncodedState).toHaveBeenCalledWith(
            1,
            JSON.stringify([1, 2, 3])
        );
        expect(controller.state.documentJson).toEqual(REMOTE_DOC);

        controller.destroy();
    });

    it('can replace the collaboration state after creation', () => {
        const controller = createYjsCollaborationController({
            documentId: 'doc-10',
            connect: false,
            createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
        });

        controller.replaceEncodedState('BQUF');

        expect(mockNativeModule.collaborationSessionReplaceEncodedState).toHaveBeenCalledWith(
            1,
            JSON.stringify([5, 5, 5])
        );
        expect(controller.state.documentJson).toEqual(REMOTE_DOC);

        controller.destroy();
    });

    it('surfaces protocol errors and clears stale peers', () => {
        const sockets: MockWebSocket[] = [];
        const onError = jest.fn();
        const controller = createYjsCollaborationController({
            documentId: 'doc-error',
            connect: false,
            createWebSocket: () => {
                const socket = new MockWebSocket();
                sockets.push(socket);
                return socket as unknown as WebSocket;
            },
            initialDocumentJson: INITIAL_DOC,
            localAwareness: {
                userId: '1',
                name: 'Alice',
                color: '#f00',
            },
            onError,
        });

        controller.connect();
        sockets[0].open();
        sockets[0].receive([9, 9, 9]);
        expect(controller.peers).toEqual(REMOTE_PEERS);

        mockNativeModule.collaborationSessionHandleMessage.mockReturnValueOnce(
            JSON.stringify({ error: 'invalid collaboration message' })
        );
        sockets[0].receive([8, 8, 8]);

        expect(controller.state.status).toBe('error');
        expect(controller.state.isConnected).toBe(false);
        expect(controller.state.lastError?.message).toBe(
            'NativeEditorBridge: invalid collaboration message'
        );
        expect(controller.peers).toEqual([]);
        expect(onError).toHaveBeenCalled();

        controller.destroy();
    });

    it('updates local awareness when hook inputs change without documentId churn', () => {
        let latest: ReturnType<typeof useYjsCollaboration> | null = null;

        function Harness({ name }: { name: string }) {
            latest = useYjsCollaboration({
                documentId: 'doc-hook',
                connect: false,
                createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
                initialDocumentJson: INITIAL_DOC,
                localAwareness: {
                    userId: '1',
                    name,
                    color: '#f00',
                },
            });
            return null;
        }

        const view = render(React.createElement(Harness, { name: 'Alice' }));

        expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenLastCalledWith(
            1,
            JSON.stringify({
                user: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
                focused: false,
            })
        );

        view.rerender(React.createElement(Harness, { name: 'Eve' }));

        expect(mockNativeModule.collaborationSessionSetLocalAwareness).toHaveBeenLastCalledWith(
            1,
            JSON.stringify({
                user: {
                    userId: '1',
                    name: 'Eve',
                    color: '#f00',
                },
                focused: false,
            })
        );
        expect(latest?.state.documentJson).toEqual(INITIAL_DOC);
    });

    it('does not recreate the collaboration session when initialDocumentJson changes', () => {
        let latest: ReturnType<typeof useYjsCollaboration> | null = null;

        function Harness({ doc }: { doc: typeof INITIAL_DOC }) {
            latest = useYjsCollaboration({
                documentId: 'doc-seed-stable',
                connect: false,
                createWebSocket: () => new MockWebSocket() as unknown as WebSocket,
                initialDocumentJson: doc,
                localAwareness: {
                    userId: '1',
                    name: 'Alice',
                    color: '#f00',
                },
            });
            return null;
        }

        const view = render(React.createElement(Harness, { doc: INITIAL_DOC }));
        expect(mockNativeModule.collaborationSessionCreate).toHaveBeenCalledTimes(1);
        expect(latest?.state.documentJson).toEqual(INITIAL_DOC);

        view.rerender(React.createElement(Harness, { doc: ALT_INITIAL_DOC }));

        expect(mockNativeModule.collaborationSessionCreate).toHaveBeenCalledTimes(1);
        expect(mockNativeModule.collaborationSessionDestroy).not.toHaveBeenCalled();
        expect(latest?.state.documentJson).toEqual(INITIAL_DOC);
    });
});
