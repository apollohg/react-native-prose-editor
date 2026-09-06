import {
    ExternalTextCompositionManager,
    type ExternalTextCompositionEndCause,
    type NativeExternalTextCompositionHandle,
} from '../ExternalTextComposition';

const active = (sessionId: string) => JSON.stringify({ version: 1, type: 'active', sessionId });
const ended = (
    sessionId: string,
    outcome: 'committed' | 'cancelled',
    cause: ExternalTextCompositionEndCause,
    text: string
) => JSON.stringify({ version: 1, type: 'ended', sessionId, outcome, cause, text });
const errored = (sessionId: string | null) =>
    JSON.stringify({ version: 1, type: 'error', sessionId, error: nativeError });
const endedWithError = (
    sessionId: string,
    outcome: 'committed' | 'cancelled',
    cause: ExternalTextCompositionEndCause,
    text: string
) =>
    JSON.stringify({
        version: 1,
        type: 'ended',
        sessionId,
        outcome,
        cause,
        text,
        error: nativeError,
    });

const nativeError = {
    domain: 'operation',
    code: 'MUTATION_REJECTED',
    message: 'Composition was rejected',
    requestId: null,
    operationIndex: null,
    limit: null,
    actual: null,
    details: null,
};

function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((next) => {
        resolve = next;
    });
    return { promise, resolve };
}

async function flushMicrotasks(): Promise<void> {
    for (let index = 0; index < 8; index += 1) await Promise.resolve();
}

function createResolvedNativeHandle() {
    const handle = {
        beginExternalTextComposition: jest.fn(async (sessionId: string) => active(sessionId)),
        updateExternalTextComposition: jest.fn(async (sessionId: string) => active(sessionId)),
        commitExternalTextComposition: jest.fn(async (sessionId: string, text: string) =>
            ended(sessionId, 'committed', 'consumer', text)
        ),
        cancelExternalTextComposition: jest.fn(
            async (sessionId: string, cause: 'consumer' | 'documentChange' | 'lifecycle') =>
                ended(sessionId, 'cancelled', cause, '')
        ),
    } satisfies NativeExternalTextCompositionHandle;
    return handle;
}

it('coalesces pending provisional values and commits the final value', async () => {
    const firstUpdate = deferred<string>();
    const update = jest
        .fn()
        .mockReturnValueOnce(firstUpdate.promise)
        .mockImplementation((sessionId: string) => Promise.resolve(active(sessionId)));
    const begin = jest.fn(async (sessionId: string) => active(sessionId));
    const native: NativeExternalTextCompositionHandle = {
        beginExternalTextComposition: begin,
        updateExternalTextComposition: update,
        commitExternalTextComposition: jest.fn(async (sessionId, text) =>
            ended(sessionId, 'committed', 'consumer', text)
        ),
        cancelExternalTextComposition: jest.fn(async (sessionId, cause) =>
            ended(sessionId, 'cancelled', cause, '')
        ),
    };
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    const sessionId = begin.mock.calls[0][0];

    const a = session.update('on');
    const b = session.update('on arr');
    const c = session.update('O/A');
    expect(update).toHaveBeenCalledTimes(1);

    firstUpdate.resolve(active(sessionId));
    await Promise.all([a, b, c]);
    expect(update).toHaveBeenCalledTimes(2);
    expect(update.mock.calls[1]).toEqual([sessionId, 'O/A']);

    await session.commit('O/A');
    expect(native.commitExternalTextComposition).toHaveBeenCalledWith(sessionId, 'O/A');
});

it('folds unsent updates into commit and settles them with the final command', async () => {
    const firstUpdate = deferred<string>();
    const commitResult = deferred<string>();
    const native = createResolvedNativeHandle();
    native.updateExternalTextComposition.mockReturnValueOnce(firstUpdate.promise);
    native.commitExternalTextComposition.mockReturnValueOnce(commitResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const first = session.update('a');
    const pending = session.update('ab');
    const commit = session.commit('abc');

    firstUpdate.resolve(active(sessionId));
    await first;
    await flushMicrotasks();
    expect(native.updateExternalTextComposition).toHaveBeenCalledTimes(1);
    expect(native.commitExternalTextComposition).toHaveBeenCalledWith(sessionId, 'abc');

    let pendingSettled = false;
    void pending.then(() => {
        pendingSettled = true;
    });
    await Promise.resolve();
    expect(pendingSettled).toBe(false);

    commitResult.resolve(ended(sessionId, 'committed', 'consumer', 'abc'));
    await expect(Promise.all([pending, commit])).resolves.toEqual([undefined, undefined]);
});

it('resolves commit when its native terminal event arrives before the bridge response', async () => {
    const commitResult = deferred<string>();
    const onEnd = jest.fn();
    const native = createResolvedNativeHandle();
    native.commitExternalTextComposition.mockReturnValueOnce(commitResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin({ onEnd });
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const settled = jest.fn();

    const commit = session.commit('O/A');
    void commit.then(() => settled('resolved'), settled);
    await flushMicrotasks();
    manager.handleNativeEnd('7', ended(sessionId, 'committed', 'consumer', 'O/A'));
    await flushMicrotasks();

    expect(settled).toHaveBeenCalledWith('resolved');
    expect(onEnd).toHaveBeenCalledTimes(1);

    commitResult.resolve(ended(sessionId, 'cancelled', 'lifecycle', 'late'));
    await flushMicrotasks();
    expect(settled).toHaveBeenCalledTimes(1);
    expect(onEnd).toHaveBeenCalledTimes(1);
});

it('resolves commit and queued updates when a matching terminal event beats a stuck update', async () => {
    const updateResult = deferred<string>();
    const native = createResolvedNativeHandle();
    native.updateExternalTextComposition.mockReturnValueOnce(updateResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const settled = jest.fn();
    const first = session.update('on');
    const queued = session.update('on arrival');

    void first.then(() => settled('first-resolved'), settled);
    void queued.then(() => settled('queued-resolved'), settled);
    void session.commit('O/A').then(() => settled('commit-resolved'), settled);
    manager.handleNativeEnd('7', ended(sessionId, 'committed', 'consumer', 'O/A'));
    await flushMicrotasks();

    expect(settled.mock.calls).toEqual([
        ['first-resolved'],
        ['queued-resolved'],
        ['commit-resolved'],
    ]);
    expect(native.commitExternalTextComposition).not.toHaveBeenCalled();
});

it('rejects commit promptly when an automatic terminal event beats a stuck update', async () => {
    const updateResult = deferred<string>();
    const native = createResolvedNativeHandle();
    native.updateExternalTextComposition.mockReturnValueOnce(updateResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const settled = jest.fn();

    void session.update('draft').catch(() => undefined);
    void session.commit('final').then(settled, (error: { code?: string }) => settled(error.code));
    manager.handleNativeEnd('7', ended(sessionId, 'committed', 'interaction', 'final'));
    await flushMicrotasks();

    expect(settled).toHaveBeenCalledWith('EXTERNAL_COMPOSITION_ENDED');
    expect(native.commitExternalTextComposition).not.toHaveBeenCalled();
});

it('rejects commit promptly with the native error when an errored terminal beats its response', async () => {
    const commitResult = deferred<string>();
    const native = createResolvedNativeHandle();
    native.commitExternalTextComposition.mockReturnValueOnce(commitResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const settled = jest.fn();

    void session.commit('final').then(settled, (error: { code?: string }) => settled(error.code));
    await flushMicrotasks();
    manager.handleNativeEnd('7', endedWithError(sessionId, 'cancelled', 'consumer', 'final'));
    await flushMicrotasks();

    expect(settled).toHaveBeenCalledWith('MUTATION_REJECTED');
});

it('rejects commit when a different-text consumer terminal beats its response', async () => {
    const commitResult = deferred<string>();
    const native = createResolvedNativeHandle();
    native.commitExternalTextComposition.mockReturnValueOnce(commitResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const settled = jest.fn();

    void session.commit('final').then(settled, (error: { code?: string }) => settled(error.code));
    await flushMicrotasks();
    manager.handleNativeEnd('7', ended(sessionId, 'committed', 'consumer', 'other'));
    await flushMicrotasks();

    expect(settled).toHaveBeenCalledWith('EXTERNAL_COMPOSITION_ENDED');
});

it('delivers automatic native termination once and rejects later updates', async () => {
    const onEnd = jest.fn();
    const native = createResolvedNativeHandle();
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin({ onEnd });
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];

    manager.handleNativeEnd('7', ended(sessionId, 'committed', 'interaction', 'O/A'));
    manager.handleNativeEnd('7', ended(sessionId, 'committed', 'interaction', 'O/A'));

    expect(onEnd).toHaveBeenCalledTimes(1);
    await expect(session.update('stale')).rejects.toMatchObject({
        code: 'EXTERNAL_COMPOSITION_ENDED',
    });
    await expect(session.cancel()).resolves.toBeUndefined();
});

it('preserves native termination that arrives before the begin response', async () => {
    const beginResult = deferred<string>();
    const onEnd = jest.fn();
    const native = createResolvedNativeHandle();
    native.beginExternalTextComposition.mockReturnValueOnce(beginResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const begin = manager.begin({ onEnd });
    await Promise.resolve();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];

    manager.handleNativeEnd('7', ended(sessionId, 'cancelled', 'interaction', 'draft'));
    beginResult.resolve(active(sessionId));

    await expect(begin).rejects.toMatchObject({ code: 'EXTERNAL_COMPOSITION_ENDED' });
    expect(onEnd).toHaveBeenCalledTimes(1);
    expect(onEnd).toHaveBeenCalledWith({
        outcome: 'cancelled',
        cause: 'interaction',
        text: 'draft',
    });
});

it.each([
    ['malformed JSON', '{'],
    ['wrong version', JSON.stringify({ version: 2, type: 'active', sessionId: '1' })],
    ['unknown key', JSON.stringify({ version: 1, type: 'active', sessionId: '1', extra: true })],
    [
        'invalid enum',
        JSON.stringify({
            version: 1,
            type: 'ended',
            sessionId: '1',
            outcome: 'saved',
            cause: 'consumer',
            text: '',
        }),
    ],
])('rejects %s in a native result', async (_label, result) => {
    const native = createResolvedNativeHandle();
    native.beginExternalTextComposition.mockResolvedValueOnce(result);
    const manager = new ExternalTextCompositionManager('7', () => native);

    await expect(manager.begin()).rejects.toMatchObject({
        name: 'NativeEditorNonRetryableError',
        code: 'EXTERNAL_COMPOSITION_RESULT_INVALID',
    });
});

it('rejects malformed native error records', async () => {
    const native = createResolvedNativeHandle();
    native.beginExternalTextComposition.mockResolvedValueOnce(
        JSON.stringify({
            version: 1,
            type: 'error',
            sessionId: null,
            error: { ...nativeError, requestId: 4 },
        })
    );

    await expect(
        new ExternalTextCompositionManager('7', () => native).begin()
    ).rejects.toMatchObject({ code: 'EXTERNAL_COMPOSITION_RESULT_INVALID' });
});

it('rejects results with the wrong session identity', async () => {
    const native = createResolvedNativeHandle();
    native.beginExternalTextComposition.mockResolvedValueOnce(active('wrong'));

    await expect(
        new ExternalTextCompositionManager('7', () => native).begin()
    ).rejects.toMatchObject({ code: 'EXTERNAL_COMPOSITION_RESULT_INVALID' });
});

it('turns returned native error records into typed exceptions', async () => {
    const native = createResolvedNativeHandle();
    native.beginExternalTextComposition.mockImplementationOnce((sessionId) =>
        JSON.stringify({ version: 1, type: 'error', sessionId, error: nativeError })
    );

    await expect(
        new ExternalTextCompositionManager('7', () => native).begin()
    ).rejects.toMatchObject({ name: 'NativeEditorOperationError', code: 'MUTATION_REJECTED' });
});

it('rejects a begin error result for another session', async () => {
    const native = createResolvedNativeHandle();
    native.beginExternalTextComposition.mockResolvedValueOnce(errored('wrong'));

    await expect(
        new ExternalTextCompositionManager('7', () => native).begin()
    ).rejects.toMatchObject({ code: 'EXTERNAL_COMPOSITION_RESULT_INVALID' });
});

it('rejects an update error result for another session', async () => {
    const native = createResolvedNativeHandle();
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    native.updateExternalTextComposition.mockResolvedValueOnce(errored('wrong'));

    await expect(session.update('draft')).rejects.toMatchObject({
        code: 'EXTERNAL_COMPOSITION_RESULT_INVALID',
    });
    await session.cancel();
});

it('rejects a commit error result for another session', async () => {
    const native = createResolvedNativeHandle();
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    native.commitExternalTextComposition.mockResolvedValueOnce(errored('wrong'));

    await expect(session.commit('final')).rejects.toMatchObject({
        code: 'EXTERNAL_COMPOSITION_RESULT_INVALID',
    });
    await session.cancel();
});

it('rejects a cancel error result for another session', async () => {
    const native = createResolvedNativeHandle();
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    native.cancelExternalTextComposition.mockResolvedValueOnce(errored('wrong'));

    await expect(session.cancel()).rejects.toMatchObject({
        code: 'EXTERNAL_COMPOSITION_RESULT_INVALID',
    });
    await expect(session.cancel()).resolves.toBeUndefined();
});

it('commits the active session before beginning a replacement', async () => {
    const native = createResolvedNativeHandle();
    const manager = new ExternalTextCompositionManager('7', () => native);
    const first = await manager.begin();
    await first.update('draft');
    const second = await manager.begin();

    const firstId = native.beginExternalTextComposition.mock.calls[0][0];
    const secondId = native.beginExternalTextComposition.mock.calls[1][0];
    expect(native.commitExternalTextComposition).toHaveBeenCalledWith(firstId, 'draft');
    expect(secondId).not.toBe(firstId);
    await expect(first.update('stale')).rejects.toMatchObject({
        code: 'EXTERNAL_COMPOSITION_ENDED',
    });
    await second.cancel();
});

it('serializes concurrent begin calls', async () => {
    const firstBegin = deferred<string>();
    const native = createResolvedNativeHandle();
    native.beginExternalTextComposition.mockReturnValueOnce(firstBegin.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const first = manager.begin();
    const second = manager.begin();

    await Promise.resolve();
    expect(native.beginExternalTextComposition).toHaveBeenCalledTimes(1);
    const firstId = native.beginExternalTextComposition.mock.calls[0][0];
    firstBegin.resolve(active(firstId));
    const firstSession = await first;
    const secondSession = await second;

    expect(native.commitExternalTextComposition).toHaveBeenCalledWith(firstId, '');
    expect(native.beginExternalTextComposition).toHaveBeenCalledTimes(2);
    await expect(firstSession.update('stale')).rejects.toMatchObject({
        code: 'EXTERNAL_COMPOSITION_ENDED',
    });
    await secondSession.cancel();
});

it('reports unsupported native handles', async () => {
    const manager = new ExternalTextCompositionManager('7', () => ({}));

    expect(manager.supports()).toBe(false);
    await expect(manager.begin()).rejects.toMatchObject({
        code: 'EXTERNAL_COMPOSITION_UNSUPPORTED',
    });
});

it('does not expose manager lifecycle and command internals', () => {
    const manager = new ExternalTextCompositionManager('7', () => null);

    expect(manager).not.toHaveProperty('owns');
    expect(manager).not.toHaveProperty('requireOwnership');
    expect(manager).not.toHaveProperty('release');
    expect(manager).not.toHaveProperty('invoke');
});

it('dispose ends the active session once and cancels native with lifecycle cause', async () => {
    const onEnd = jest.fn();
    const native = createResolvedNativeHandle();
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin({ onEnd });
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];

    manager.dispose();
    manager.dispose();

    expect(onEnd).toHaveBeenCalledTimes(1);
    expect(onEnd).toHaveBeenCalledWith({
        outcome: 'cancelled',
        cause: 'lifecycle',
        text: '',
    });
    expect(native.cancelExternalTextComposition).toHaveBeenCalledTimes(1);
    expect(native.cancelExternalTextComposition).toHaveBeenCalledWith(sessionId, 'lifecycle');
    await expect(session.commit('late')).rejects.toMatchObject({
        code: 'EXTERNAL_COMPOSITION_ENDED',
    });
});

it('cancels the active session for a document change', async () => {
    const native = createResolvedNativeHandle();
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];

    await manager.cancelForDocumentChange();
    await manager.cancelForDocumentChange();

    expect(native.cancelExternalTextComposition).toHaveBeenCalledTimes(1);
    expect(native.cancelExternalTextComposition).toHaveBeenCalledWith(sessionId, 'documentChange');
    await expect(session.update('late')).rejects.toMatchObject({
        code: 'EXTERNAL_COMPOSITION_ENDED',
    });
});

it('resolves cancel when automatic native termination wins the race', async () => {
    const cancelResult = deferred<string>();
    const onEnd = jest.fn();
    const native = createResolvedNativeHandle();
    native.cancelExternalTextComposition.mockReturnValueOnce(cancelResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin({ onEnd });
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const cancel = session.cancel();

    manager.handleNativeEnd('7', ended(sessionId, 'cancelled', 'interaction', ''));
    cancelResult.resolve(ended(sessionId, 'cancelled', 'consumer', ''));

    await expect(cancel).resolves.toBeUndefined();
    expect(onEnd).toHaveBeenCalledTimes(1);
    await expect(session.cancel()).resolves.toBeUndefined();
});

it('settles cancel and coalesced updates without waiting for native update', async () => {
    const updateResult = deferred<string>();
    const native = createResolvedNativeHandle();
    native.updateExternalTextComposition.mockReturnValueOnce(updateResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const settledUpdates: Array<string | undefined> = [];
    const updates = [session.update('a'), session.update('ab'), session.update('abc')];
    for (const update of updates) {
        void update.then(
            () => settledUpdates.push('resolved'),
            (error: { code?: string }) => settledUpdates.push(error.code)
        );
    }
    const cancel = session.cancel();
    const cancelSettled = jest.fn();
    void cancel.then(cancelSettled, cancelSettled);

    manager.handleNativeEnd('7', ended(sessionId, 'cancelled', 'interaction', ''));
    await flushMicrotasks();

    expect(cancelSettled).toHaveBeenCalledWith(undefined);
    expect(settledUpdates).toEqual([
        'EXTERNAL_COMPOSITION_ENDED',
        'EXTERNAL_COMPOSITION_ENDED',
        'EXTERNAL_COMPOSITION_ENDED',
    ]);
    expect(native.cancelExternalTextComposition).not.toHaveBeenCalled();
});

it('settles detached update waiters without waiting for native cancel', async () => {
    const updateResult = deferred<string>();
    const cancelResult = deferred<string>();
    const native = createResolvedNativeHandle();
    native.updateExternalTextComposition.mockReturnValueOnce(updateResult.promise);
    native.cancelExternalTextComposition.mockReturnValueOnce(cancelResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const first = session.update('a');
    const pendingSettled = jest.fn();
    void session.update('ab').then(
        () => pendingSettled('resolved'),
        (error: { code?: string }) => pendingSettled(error.code)
    );
    const cancel = session.cancel();
    const cancelSettled = jest.fn();
    void cancel.then(cancelSettled, cancelSettled);

    updateResult.resolve(active(sessionId));
    await first;
    await flushMicrotasks();
    expect(native.cancelExternalTextComposition).toHaveBeenCalledTimes(1);
    manager.handleNativeEnd('7', ended(sessionId, 'cancelled', 'interaction', ''));
    await flushMicrotasks();

    expect(cancelSettled).toHaveBeenCalledWith(undefined);
    expect(pendingSettled).toHaveBeenCalledWith('EXTERNAL_COMPOSITION_ENDED');
});

it('does not activate a begin result that arrives after disposal', async () => {
    const beginResult = deferred<string>();
    const native = createResolvedNativeHandle();
    native.beginExternalTextComposition.mockReturnValueOnce(beginResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const begin = manager.begin();
    await Promise.resolve();
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];

    manager.dispose();
    beginResult.resolve(active(sessionId));

    await expect(begin).rejects.toMatchObject({ code: 'EXTERNAL_COMPOSITION_ENDED' });
    expect(native.cancelExternalTextComposition).toHaveBeenCalledWith(sessionId, 'lifecycle');
});

it('does not accept a late update result after disposal', async () => {
    const updateResult = deferred<string>();
    const onEnd = jest.fn();
    const native = createResolvedNativeHandle();
    native.updateExternalTextComposition.mockReturnValueOnce(updateResult.promise);
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin({ onEnd });
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];
    const update = session.update('draft');

    manager.dispose();
    updateResult.resolve(active(sessionId));

    await expect(update).rejects.toMatchObject({ code: 'EXTERNAL_COMPOSITION_ENDED' });
    expect(onEnd).toHaveBeenCalledTimes(1);
});

it('ignores native end events for other editors and sessions', async () => {
    const onEnd = jest.fn();
    const native = createResolvedNativeHandle();
    const manager = new ExternalTextCompositionManager('7', () => native);
    const session = await manager.begin({ onEnd });
    const sessionId = native.beginExternalTextComposition.mock.calls[0][0];

    manager.handleNativeEnd('8', ended(sessionId, 'cancelled', 'interaction', ''));
    manager.handleNativeEnd('7', ended('other', 'cancelled', 'interaction', ''));
    expect(onEnd).not.toHaveBeenCalled();
    await session.cancel();
});
