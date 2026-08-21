import {
    NativeEditorV2LifecycleError,
    NativeEditorV2NonRetryableError,
    nativeEditorV2ErrorToException,
    normalizeNativeEditorV2Error,
    type NativeEditorV2Error,
    type NativeEditorV2ErrorBase,
} from './NativeEditorBoundaryError';

export type ExternalTextCompositionEndCause =
    | 'consumer'
    | 'interaction'
    | 'documentChange'
    | 'lifecycle';

export interface ExternalTextCompositionEndEvent {
    outcome: 'committed' | 'cancelled';
    cause: ExternalTextCompositionEndCause;
    text: string;
    error?: NativeEditorV2ErrorBase;
}

export interface ExternalTextCompositionOptions {
    onEnd?: (event: ExternalTextCompositionEndEvent) => void;
}

export interface ExternalTextCompositionSession {
    update(text: string): Promise<void>;
    commit(finalText: string): Promise<void>;
    cancel(): Promise<void>;
}

export interface NativeExternalTextCompositionHandle {
    beginExternalTextComposition?(sessionId: string): Promise<string> | string;
    updateExternalTextComposition?(sessionId: string, text: string): Promise<string> | string;
    commitExternalTextComposition?(sessionId: string, text: string): Promise<string> | string;
    cancelExternalTextComposition?(
        sessionId: string,
        cause: 'consumer' | 'documentChange' | 'lifecycle'
    ): Promise<string> | string;
}

type NativeCompositionResult =
    | { version: 1; type: 'active'; sessionId: string }
    | {
          version: 1;
          type: 'ended';
          sessionId: string;
          outcome: 'committed' | 'cancelled';
          cause: ExternalTextCompositionEndCause;
          text: string;
          error?: NativeEditorV2Error;
      }
    | { version: 1; type: 'error'; sessionId: string | null; error: NativeEditorV2Error };

type NativeCompositionEndResult = Extract<NativeCompositionResult, { type: 'ended' }>;
type Waiter = { resolve: () => void; reject: (error: unknown) => void };
type PendingUpdate = { text: string; waiters: Waiter[] };
type TerminalRace<T> =
    | { type: 'completed'; value: T }
    | { type: 'failed'; error: unknown }
    | { type: 'terminal'; result: NativeCompositionEndResult };

interface ManagedExternalTextCompositionHost {
    owns(session: ManagedExternalTextCompositionSession): boolean;
    release(session: ManagedExternalTextCompositionSession): void;
    update(sessionId: string, text: string): Promise<NativeCompositionResult>;
    commit(sessionId: string, text: string): Promise<NativeCompositionResult>;
    cancel(
        sessionId: string,
        cause: 'consumer' | 'documentChange'
    ): Promise<NativeCompositionResult>;
}

let nextExternalCompositionId = 1n;

function allocateExternalCompositionId(): string {
    const id = nextExternalCompositionId.toString(10);
    nextExternalCompositionId += 1n;
    return id;
}

export function createExternalCompositionLifecycleError(
    code: string,
    message: string
): NativeEditorV2LifecycleError {
    return new NativeEditorV2LifecycleError({
        domain: 'lifecycle',
        code,
        message,
        requestId: null,
        operationIndex: null,
        limit: null,
        actual: null,
        details: null,
    });
}

function invalidResultError(): NativeEditorV2NonRetryableError {
    return new NativeEditorV2NonRetryableError({
        domain: 'boundary',
        code: 'EXTERNAL_COMPOSITION_RESULT_INVALID',
        message: 'Native external text composition returned an invalid result',
        requestId: null,
        operationIndex: null,
        limit: null,
        actual: null,
        details: null,
    });
}

function endedLifecycleError(): NativeEditorV2LifecycleError {
    return createExternalCompositionLifecycleError(
        'EXTERNAL_COMPOSITION_ENDED',
        'The external text composition session has ended'
    );
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return value != null && typeof value === 'object' && !Array.isArray(value);
}

function hasExactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
    const actual = Object.keys(value);
    return actual.length === keys.length && actual.every((key) => keys.includes(key));
}

const ERROR_KEYS = [
    'domain',
    'code',
    'message',
    'requestId',
    'operationIndex',
    'limit',
    'actual',
    'details',
] as const;

function parseError(value: unknown): NativeEditorV2Error | null {
    if (!isRecord(value) || !hasExactKeys(value, ERROR_KEYS)) return null;
    return normalizeNativeEditorV2Error({ error: value });
}

function parseNativeCompositionResult(resultJson: string): NativeCompositionResult {
    let value: unknown;
    try {
        value = JSON.parse(resultJson);
    } catch {
        throw invalidResultError();
    }
    if (!isRecord(value) || value.version !== 1 || typeof value.type !== 'string') {
        throw invalidResultError();
    }

    if (value.type === 'active') {
        if (
            !hasExactKeys(value, ['version', 'type', 'sessionId']) ||
            typeof value.sessionId !== 'string'
        ) {
            throw invalidResultError();
        }
        return { version: 1, type: 'active', sessionId: value.sessionId };
    }

    if (value.type === 'ended') {
        const keys = Object.prototype.hasOwnProperty.call(value, 'error')
            ? ['version', 'type', 'sessionId', 'outcome', 'cause', 'text', 'error']
            : ['version', 'type', 'sessionId', 'outcome', 'cause', 'text'];
        const validOutcome = value.outcome === 'committed' || value.outcome === 'cancelled';
        const validCause =
            value.cause === 'consumer' ||
            value.cause === 'interaction' ||
            value.cause === 'documentChange' ||
            value.cause === 'lifecycle';
        const error = keys.includes('error') ? parseError(value.error) : undefined;
        if (
            !hasExactKeys(value, keys) ||
            typeof value.sessionId !== 'string' ||
            !validOutcome ||
            !validCause ||
            typeof value.text !== 'string' ||
            (keys.includes('error') && error == null)
        ) {
            throw invalidResultError();
        }
        return {
            version: 1,
            type: 'ended',
            sessionId: value.sessionId,
            outcome: value.outcome as 'committed' | 'cancelled',
            cause: value.cause as ExternalTextCompositionEndCause,
            text: value.text,
            ...(error == null ? {} : { error }),
        };
    }

    if (value.type === 'error') {
        const error = parseError(value.error);
        if (
            !hasExactKeys(value, ['version', 'type', 'sessionId', 'error']) ||
            (value.sessionId !== null && typeof value.sessionId !== 'string') ||
            error == null
        ) {
            throw invalidResultError();
        }
        return { version: 1, type: 'error', sessionId: value.sessionId, error };
    }

    throw invalidResultError();
}

function requireActiveResult(result: NativeCompositionResult, sessionId: string): void {
    if (result.type === 'error') {
        if (result.sessionId !== null && result.sessionId !== sessionId) {
            throw invalidResultError();
        }
        throw nativeEditorV2ErrorToException(result.error);
    }
    if (result.type !== 'active' || result.sessionId !== sessionId) {
        throw invalidResultError();
    }
}

function requireEndResult(
    result: NativeCompositionResult,
    sessionId: string
): NativeCompositionEndResult {
    if (result.type === 'error') {
        if (result.sessionId !== null && result.sessionId !== sessionId) {
            throw invalidResultError();
        }
        throw nativeEditorV2ErrorToException(result.error);
    }
    if (result.type !== 'ended' || result.sessionId !== sessionId) {
        throw invalidResultError();
    }
    return result;
}

function settleWaiters(waiters: Waiter[], error?: unknown): void {
    for (const waiter of waiters) {
        if (error === undefined) waiter.resolve();
        else waiter.reject(error);
    }
}

class ManagedExternalTextCompositionSession implements ExternalTextCompositionSession {
    latestText = '';
    private terminal = false;
    private terminalResult: NativeCompositionEndResult | null = null;
    private terminalError: NativeEditorV2ErrorBase | null = null;
    private resolveTerminal!: (result: NativeCompositionEndResult) => void;
    private readonly terminalCompletion = new Promise<NativeCompositionEndResult>((resolve) => {
        this.resolveTerminal = resolve;
    });
    private closing: 'commit' | 'cancel' | null = null;
    private updateInFlight: Promise<void> | null = null;
    private inFlightWaiters: Waiter[] = [];
    private pendingUpdate: PendingUpdate | null = null;
    private cancelPromise: Promise<void> | null = null;

    constructor(
        readonly id: string,
        private readonly host: ManagedExternalTextCompositionHost,
        private readonly options: ExternalTextCompositionOptions
    ) {}

    update(text: string): Promise<void> {
        if (!this.canMutate()) return Promise.reject(endedLifecycleError());
        this.latestText = text;
        return new Promise<void>((resolve, reject) => {
            const waiter = { resolve, reject };
            if (this.updateInFlight != null) {
                if (this.pendingUpdate == null) {
                    this.pendingUpdate = { text, waiters: [waiter] };
                } else {
                    this.pendingUpdate.text = text;
                    this.pendingUpdate.waiters.push(waiter);
                }
                return;
            }
            this.startUpdate(text, [waiter]);
        });
    }

    async commit(finalText: string): Promise<void> {
        if (!this.canMutate()) throw endedLifecycleError();
        this.closing = 'commit';
        this.latestText = finalText;
        if (this.updateInFlight != null) {
            const update = await this.raceWithTerminal(this.updateInFlight);
            if (update.type === 'terminal') {
                this.requireSuccessfulConsumerCommit(update.result, finalText);
                return;
            }
            if (update.type === 'failed') throw update.error;
        }
        if (this.terminalResult != null) {
            this.requireSuccessfulConsumerCommit(this.terminalResult, finalText);
            return;
        }
        if (!this.canFinish()) throw endedLifecycleError();

        const foldedWaiters = this.takePendingWaiters();
        const commit = await this.raceWithTerminal(this.host.commit(this.id, finalText));
        if (commit.type === 'terminal') {
            const error = this.explicitCommitError(commit.result, finalText);
            settleWaiters(foldedWaiters, error);
            if (error) throw error;
            return;
        }
        if (commit.type === 'failed') {
            settleWaiters(foldedWaiters, commit.error);
            if (!this.terminal && this.host.owns(this)) this.closing = null;
            throw commit.error;
        }
        try {
            this.requireOwnership();
            const endResult = requireEndResult(commit.value, this.id);
            const endError = this.explicitCommitError(endResult, finalText);
            this.finish(endResult);
            settleWaiters(foldedWaiters, endError);
            if (endError) throw endError;
        } catch (error) {
            settleWaiters(foldedWaiters, error);
            if (!this.terminal && this.host.owns(this)) this.closing = null;
            throw error;
        }
    }

    cancel(): Promise<void> {
        if (this.terminal) return Promise.resolve();
        if (this.cancelPromise != null) return this.cancelPromise;
        if (!this.canMutate()) return Promise.reject(endedLifecycleError());
        this.cancelPromise = this.cancelWithCause('consumer');
        return this.cancelPromise;
    }

    cancelForDocumentChange(): Promise<void> {
        if (this.terminal) return Promise.resolve();
        if (this.cancelPromise != null) return this.cancelPromise;
        if (!this.canMutate()) return Promise.reject(endedLifecycleError());
        this.cancelPromise = this.cancelWithCause('documentChange');
        return this.cancelPromise;
    }

    finish(result: NativeCompositionEndResult): void {
        if (this.terminal) return;
        this.terminal = true;
        this.terminalResult = result;
        const error = result.error
            ? nativeEditorV2ErrorToException(result.error)
            : endedLifecycleError();
        this.terminalError = error;
        const waiterError =
            this.closing === 'commit' && this.explicitCommitError(result, this.latestText) == null
                ? undefined
                : error;
        settleWaiters(this.inFlightWaiters, waiterError);
        this.inFlightWaiters = [];
        if (this.pendingUpdate != null) {
            settleWaiters(this.pendingUpdate.waiters, waiterError);
            this.pendingUpdate = null;
        }
        this.host.release(this);
        this.resolveTerminal(result);
        this.options.onEnd?.({
            outcome: result.outcome,
            cause: result.cause,
            text: result.text,
            ...(result.error == null ? {} : { error }),
        });
    }

    private canMutate(): boolean {
        return !this.terminal && this.closing == null && this.host.owns(this);
    }

    private canFinish(): boolean {
        return !this.terminal && this.closing != null && this.host.owns(this);
    }

    private startUpdate(text: string, waiters: Waiter[]): void {
        this.inFlightWaiters = waiters;
        this.updateInFlight = this.performUpdate(text, waiters);
    }

    private async performUpdate(text: string, waiters: Waiter[]): Promise<void> {
        try {
            const result = await this.host.update(this.id, text);
            this.requireOwnership();
            requireActiveResult(result, this.id);
            settleWaiters(waiters);
        } catch (error) {
            settleWaiters(waiters, error);
        } finally {
            if (this.inFlightWaiters === waiters) this.inFlightWaiters = [];
            this.updateInFlight = null;
            if (!this.terminal && this.closing == null && this.pendingUpdate != null) {
                const pending = this.pendingUpdate;
                this.pendingUpdate = null;
                this.startUpdate(pending.text, pending.waiters);
            }
        }
    }

    private takePendingWaiters(): Waiter[] {
        const waiters = this.pendingUpdate?.waiters ?? [];
        this.pendingUpdate = null;
        return waiters;
    }

    private requireOwnership(): void {
        if (!this.host.owns(this)) throw endedLifecycleError();
    }

    private raceWithTerminal<T>(work: Promise<T>): Promise<TerminalRace<T>> {
        return Promise.race([
            work.then<TerminalRace<T>, TerminalRace<T>>(
                (value) => ({ type: 'completed', value }),
                (error: unknown) => ({ type: 'failed', error })
            ),
            this.terminalCompletion.then<TerminalRace<T>>((result) => ({
                type: 'terminal',
                result,
            })),
        ]);
    }

    private explicitCommitError(
        result: NativeCompositionEndResult,
        finalText: string
    ): NativeEditorV2ErrorBase | undefined {
        if (
            result.outcome === 'committed' &&
            result.cause === 'consumer' &&
            result.text === finalText &&
            result.error == null
        ) {
            return undefined;
        }
        return result.error ? nativeEditorV2ErrorToException(result.error) : endedLifecycleError();
    }

    private requireSuccessfulConsumerCommit(
        result: NativeCompositionEndResult,
        finalText: string
    ): void {
        const error = this.explicitCommitError(result, finalText);
        if (error) throw error;
    }

    private settleTerminalWaiters(waiters: Waiter[]): void {
        settleWaiters(waiters, this.terminalError ?? endedLifecycleError());
    }

    private async cancelWithCause(cause: 'consumer' | 'documentChange'): Promise<void> {
        this.closing = 'cancel';
        if (this.updateInFlight != null) {
            const update = await this.raceWithTerminal(this.updateInFlight);
            if (update.type === 'terminal') return;
            if (update.type === 'failed') throw update.error;
        }
        if (!this.canFinish()) return;
        const pendingWaiters = this.takePendingWaiters();
        const cancellation = await this.raceWithTerminal(this.host.cancel(this.id, cause));
        if (cancellation.type === 'terminal') {
            this.settleTerminalWaiters(pendingWaiters);
            return;
        }
        if (cancellation.type === 'failed') {
            settleWaiters(pendingWaiters, cancellation.error);
            if (this.host.owns(this)) {
                this.closing = null;
                this.cancelPromise = null;
            }
            throw cancellation.error;
        }
        try {
            this.requireOwnership();
            const endResult = requireEndResult(cancellation.value, this.id);
            const endError = endResult.error
                ? nativeEditorV2ErrorToException(endResult.error)
                : undefined;
            this.finish(endResult);
            settleWaiters(pendingWaiters, endError);
            if (endError) throw endError;
        } catch (error) {
            settleWaiters(pendingWaiters, error);
            if (!this.terminal && this.host.owns(this)) {
                this.closing = null;
                this.cancelPromise = null;
            }
            throw error;
        }
    }
}

export class ExternalTextCompositionManager {
    private active: ManagedExternalTextCompositionSession | null = null;
    private pendingBegin: ManagedExternalTextCompositionSession | null = null;
    private pendingEnd: NativeCompositionEndResult | null = null;
    private disposed = false;
    private beginTail: Promise<void> = Promise.resolve();
    private readonly sessionHost: ManagedExternalTextCompositionHost = {
        owns: (session) => !this.disposed && this.active === session,
        release: (session) => {
            if (this.active === session) this.active = null;
        },
        update: (sessionId, text) => this.invokeUpdate(sessionId, text),
        commit: (sessionId, text) => this.invokeCommit(sessionId, text),
        cancel: (sessionId, cause) => this.invokeCancel(sessionId, cause),
    };

    constructor(
        private readonly editorId: string,
        private readonly getNativeHandle: () => NativeExternalTextCompositionHandle | null
    ) {}

    supports(): boolean {
        const handle = this.getNativeHandle();
        return (
            typeof handle?.beginExternalTextComposition === 'function' &&
            typeof handle.updateExternalTextComposition === 'function' &&
            typeof handle.commitExternalTextComposition === 'function' &&
            typeof handle.cancelExternalTextComposition === 'function'
        );
    }

    begin(options: ExternalTextCompositionOptions = {}): Promise<ExternalTextCompositionSession> {
        const pending = this.beginTail.then(() => this.beginSerialized(options));
        this.beginTail = pending.then(
            () => undefined,
            () => undefined
        );
        return pending;
    }

    handleNativeEnd(editorId: string, resultJson: string): void {
        if (editorId !== this.editorId) return;
        const result = parseNativeCompositionResult(resultJson);
        if (result.type !== 'ended') return;
        if (result.sessionId === this.active?.id) {
            this.active.finish(result);
            return;
        }
        if (result.sessionId === this.pendingBegin?.id && this.pendingEnd == null) {
            this.pendingEnd = result;
            this.pendingBegin.finish(result);
        }
    }

    cancelForDocumentChange(): Promise<void> {
        return this.active?.cancelForDocumentChange() ?? Promise.resolve();
    }

    dispose(): void {
        if (this.disposed) return;
        this.disposed = true;
        const active = this.active;
        if (!active) return;
        active.finish({
            version: 1,
            type: 'ended',
            sessionId: active.id,
            outcome: 'cancelled',
            cause: 'lifecycle',
            text: active.latestText,
        });
        void this.invokeCancel(active.id, 'lifecycle').catch(() => undefined);
    }

    private unsupportedError(): NativeEditorV2LifecycleError {
        return createExternalCompositionLifecycleError(
            'EXTERNAL_COMPOSITION_UNSUPPORTED',
            'The mounted native editor does not support external text composition'
        );
    }

    private async invokeBegin(sessionId: string): Promise<NativeCompositionResult> {
        const handle = this.getNativeHandle();
        const method = handle?.beginExternalTextComposition;
        if (typeof method !== 'function') {
            throw this.unsupportedError();
        }
        const resultJson = await method.call(handle, sessionId);
        return parseNativeCompositionResult(resultJson);
    }

    private async invokeUpdate(sessionId: string, text: string): Promise<NativeCompositionResult> {
        const handle = this.getNativeHandle();
        const method = handle?.updateExternalTextComposition;
        if (typeof method !== 'function') {
            throw this.unsupportedError();
        }
        const resultJson = await method.call(handle, sessionId, text);
        return parseNativeCompositionResult(resultJson);
    }

    private async invokeCommit(sessionId: string, text: string): Promise<NativeCompositionResult> {
        const handle = this.getNativeHandle();
        const method = handle?.commitExternalTextComposition;
        if (typeof method !== 'function') {
            throw this.unsupportedError();
        }
        const resultJson = await method.call(handle, sessionId, text);
        return parseNativeCompositionResult(resultJson);
    }

    private async invokeCancel(
        sessionId: string,
        cause: 'consumer' | 'documentChange' | 'lifecycle'
    ): Promise<NativeCompositionResult> {
        const handle = this.getNativeHandle();
        const method = handle?.cancelExternalTextComposition;
        if (typeof method !== 'function') {
            throw this.unsupportedError();
        }
        const resultJson = await method.call(handle, sessionId, cause);
        return parseNativeCompositionResult(resultJson);
    }

    private async beginSerialized(
        options: ExternalTextCompositionOptions
    ): Promise<ExternalTextCompositionSession> {
        if (this.disposed) throw endedLifecycleError();
        if (!this.supports()) {
            throw this.unsupportedError();
        }
        if (this.active) await this.active.commit(this.active.latestText);
        if (this.disposed) throw endedLifecycleError();

        const session = new ManagedExternalTextCompositionSession(
            allocateExternalCompositionId(),
            this.sessionHost,
            options
        );
        this.pendingBegin = session;
        this.pendingEnd = null;
        try {
            const result = await this.invokeBegin(session.id);
            if (this.disposed) {
                void this.invokeCancel(session.id, 'lifecycle').catch(() => undefined);
                throw endedLifecycleError();
            }
            if (this.pendingBegin !== session) throw endedLifecycleError();
            requireActiveResult(result, session.id);
            if (this.pendingEnd != null) throw endedLifecycleError();
            this.active = session;
            return session;
        } finally {
            if (this.pendingBegin === session) {
                this.pendingBegin = null;
                this.pendingEnd = null;
            }
        }
    }
}
