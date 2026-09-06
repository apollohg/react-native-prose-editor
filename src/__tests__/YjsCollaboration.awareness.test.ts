import './helpers/YjsCollaborationFixture';
import {
    TRANSPORT_URL,
    SNAPSHOT_DOC,
    ALICE,
    remotePeer,
    localAwarenessIntent,
    runtime,
    createRoomHandle,
    setupController,
    synchronize,
    awarenessPayload,
} from './helpers/YjsCollaborationFixture';

import {
    fakeDocForText,
    V2_FAKE_AWARENESS_FRAME,
    V2_FAKE_UPDATE_FRAME,
} from './helpers/nativeEditorV2Fake';
import { act, renderHook } from '@testing-library/react-native';
import { useYjsCollaboration } from '../YjsCollaboration';

describe('YjsCollaboration (native-transport controller)', () => {
    it('reports peers from the native projection and derives remote selections', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const { result } = renderHook(() =>
            useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                transport: { url: TRANSPORT_URL, connect: true },
                localAwareness: ALICE,
            })
        );
        act(() => {
            synchronize(handle);
        });
        act(() => {
            runtime.pushRemotePeers(handle.editorId, [
                remotePeer({
                    state: {
                        state: {
                            user: { userId: '2', name: 'Bob', color: '#00f' },
                            // A caller-authored scalar selection is never a cursor.
                            selection: { anchor: 90, head: 91 },
                        },
                        focused: true,
                    },
                    cursor: { anchor: 4, head: 9 },
                }),
                remotePeer({
                    clientId: '43',
                    cursor: null,
                    state: {
                        state: {
                            user: { userId: '3', name: 'Carol', color: '#0a0' },
                            selection: { anchor: 40, head: 41 },
                        },
                        focused: true,
                    },
                }),
            ]);
            runtime.transportReceive(handle.editorId, V2_FAKE_AWARENESS_FRAME);
        });

        expect(result.current.editorBindings.remoteSelections).toEqual([
            {
                clientId: '42',
                anchor: 4,
                head: 9,
                color: '#00f',
                name: 'Bob',
                avatarUrl: undefined,
                isFocused: true,
            },
        ]);
    });

    it('falls back to a default color and omits an absent name', () => {
        const setup = setupController({ handle: createRoomHandle({ withSnapshot: true }) });
        setup.controller.connect();
        synchronize(setup.handle);
        runtime.pushRemotePeers(setup.handle.editorId, [
            remotePeer({ state: { state: {}, focused: false } }),
        ]);
        runtime.transportReceive(setup.handle.editorId, V2_FAKE_AWARENESS_FRAME);

        expect(setup.peersLog.at(-1)).toHaveLength(1);
        expect(setup.controller.peers[0].clientId).toBe('42');
    });

    it('publishes the local awareness intent through Rust with no TypeScript clock bookkeeping', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        expect(awarenessPayload()).toEqual(localAwarenessIntent());
        // Clocks, tombstones, and renewal deadlines are Rust-owned: the
        // published intent carries none of them.
        expect(JSON.stringify(awarenessPayload())).not.toContain('clock');
        expect(setup.controller.state.documentJson).toEqual(SNAPSHOT_DOC);
    });

    it('omits undefined optional fields from initial local awareness', () => {
        const setup = setupController({
            localAwareness: { ...ALICE, avatarUrl: undefined },
        });

        expect(setup.errors).toEqual([]);
        expect(awarenessPayload()).toEqual(localAwarenessIntent());
    });

    it('omits nested undefined fields from local awareness metadata', () => {
        const setup = setupController({
            localAwareness: {
                ...ALICE,
                extra: { team: 'editor', role: undefined },
            },
        });

        expect(setup.errors).toEqual([]);
        expect(awarenessPayload()).toEqual(
            localAwarenessIntent({ user: { ...ALICE, extra: { team: 'editor' } } })
        );
    });

    it('never restates a document position on a focus-only update', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.connect();
        synchronize(setup.handle);
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });
        expect(awarenessPayload()).toMatchObject({
            selection: { type: 'text', anchor: 2, head: 5 },
        });

        // Focus and blur state no position at all, so the Rust-owned sticky
        // cursor is retained rather than re-resolved against a document
        // that may have moved underneath the last observed offsets.
        setup.controller.handleFocusChange(true);
        expect(awarenessPayload()).not.toHaveProperty('selection');
        setup.controller.handleFocusChange(false);
        expect(awarenessPayload()).not.toHaveProperty('selection');
        expect(setup.errors).toEqual([]);
    });

    it('blur cannot fail after the document invalidates the last observed selection', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.connect();
        synchronize(setup.handle);
        // A caret near the end of "snapshot" is valid when it is observed.
        setup.controller.handleSelectionChange({ type: 'text', anchor: 8, head: 8 });
        setup.controller.handleFocusChange(true);

        // A remote peer then shrinks the document under that offset, which
        // is exactly the state the reported blur crash was reproduced from.
        runtime.pushRemoteDoc(setup.handle.editorId, fakeDocForText('hi'));
        runtime.transportReceive(setup.handle.editorId, V2_FAKE_UPDATE_FRAME);

        expect(() => setup.controller.handleFocusChange(false)).not.toThrow();
        expect(awarenessPayload()).toEqual({ state: { user: ALICE }, focused: false });
        expect(setup.errors).toEqual([]);
    });

    it('reports a refused explicit selection instead of throwing into the caller', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.connect();
        synchronize(setup.handle);

        // An explicitly stated position outside the document is still a
        // caller contract violation — reported, never silently accepted.
        expect(() =>
            setup.controller.handleSelectionChange({ type: 'text', anchor: 999, head: 999 })
        ).not.toThrow();
        expect(setup.errors).toHaveLength(1);
        expect(setup.errors[0].message).toContain('outside the current document');
        expect(runtime.session(setup.handle.editorId).desiredAwareness).toEqual(
            localAwarenessIntent()
        );
    });

    it('composes application state, focus, and document selection into the local intent', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });
        expect(awarenessPayload()).toEqual({
            state: { user: ALICE },
            focused: false,
            selection: { type: 'text', anchor: 2, head: 5 },
        });

        setup.controller.handleFocusChange(true);
        expect(awarenessPayload()).toMatchObject({ focused: true });

        setup.controller.updateLocalAwareness({ user: { ...ALICE, name: 'Alice II' } });
        expect(awarenessPayload()).toMatchObject({
            state: { user: { ...ALICE, name: 'Alice II' } },
        });
    });

    it('omits undefined optional fields from local awareness updates', () => {
        const setup = setupController({
            localAwareness: { ...ALICE, avatarUrl: 'https://example.test/alice.png' },
        });

        setup.controller.updateLocalAwareness({
            user: { ...ALICE, avatarUrl: undefined },
        });

        expect(setup.errors).toEqual([]);
        expect(awarenessPayload()).toEqual(localAwarenessIntent());
    });

    it('normalizes node and all selections to a cursorless awareness intent', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        setup.controller.handleSelectionChange({ type: 'text', anchor: 2, head: 5 });

        // A non-text selection is an explicit "no cursor", not a silent
        // retention of the last text position.
        setup.controller.handleSelectionChange({ type: 'node', pos: 3 });
        expect(awarenessPayload()).toEqual({
            state: { user: ALICE },
            focused: false,
            selection: null,
        });

        setup.controller.handleSelectionChange({ type: 'text', anchor: 3, head: 4 });
        setup.controller.handleSelectionChange({ type: 'all' });
        expect(awarenessPayload()).toEqual({
            state: { user: ALICE },
            focused: false,
            selection: null,
        });
    });

    it('setting identical awareness twice emits no publish', () => {
        const setup = setupController({
            handle: createRoomHandle({ withSnapshot: true }),
            localAwareness: ALICE,
        });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);

        setup.controller.updateLocalAwareness({ user: { ...ALICE } });
        setup.controller.updateLocalAwareness({ focused: false });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(1);

        setup.controller.updateLocalAwareness({ focused: true });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(2);
    });

    it('withdraws local awareness when a controller omits localAwareness', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const first = setupController({ handle, localAwareness: ALICE });
        first.controller.connect();
        synchronize(handle);
        expect(runtime.session(handle.editorId).desiredAwareness).toEqual(localAwarenessIntent());
        first.controller.destroy();

        runtime.module.editorV2CollaborationSetAwareness.mockClear();
        setupController({ handle });
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenLastCalledWith(
            handle.editorId,
            'null'
        );
        expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();
    });

    it('keeps focus and selection hooks inert after awareness is withdrawn', () => {
        const handle = createRoomHandle({ withSnapshot: true });
        const setup = setupController({ handle, localAwareness: ALICE });
        setup.controller.applyLocalAwarenessOption(undefined);
        const publishesAfterWithdrawal =
            runtime.module.editorV2CollaborationSetAwareness.mock.calls.length;

        setup.controller.handleFocusChange(true);
        setup.controller.handleFocusChange(false);
        setup.controller.handleSelectionChange({ type: 'text', anchor: 1, head: 3 });

        // Presence must not resurrect itself as an anonymous entry.
        expect(runtime.module.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(
            publishesAfterWithdrawal
        );
        expect(runtime.session(handle.editorId).desiredAwareness).toBeNull();

        // An explicit user restores it.
        setup.controller.updateLocalAwareness({ user: ALICE });
        expect(runtime.session(handle.editorId).desiredAwareness).toEqual(
            localAwarenessIntent({ user: ALICE }, false)
        );
    });

    it('reports an awareness intent above the peer-byte limit without throwing', () => {
        const setup = setupController({
            handle: createRoomHandle({
                withSnapshot: true,
                limits: { collaboration: { maxAwarenessPeerBytes: 128 } },
            }),
            localAwareness: ALICE,
        });
        const acceptedIntent = runtime.session(setup.handle.editorId).desiredAwareness;

        // Presence is ambient UI state: a refusal is reported, never thrown
        // into the caller, and the last accepted intent stays in force.
        expect(() =>
            setup.controller.updateLocalAwareness({
                user: { ...ALICE, name: 'A'.repeat(256) },
            })
        ).not.toThrow();
        expect(setup.errors).toHaveLength(1);
        expect(setup.controller.state.lastError).toBe(setup.errors[0]);
        expect(runtime.session(setup.handle.editorId).desiredAwareness).toEqual(acceptedIntent);

        // The rejected candidate was discarded, so a later valid change is
        // still published rather than being deduped against it.
        setup.controller.updateLocalAwareness({ user: { ...ALICE, name: 'Alice II' } });
        expect(awarenessPayload()).toMatchObject({
            state: { user: { ...ALICE, name: 'Alice II' } },
        });
    });
});
