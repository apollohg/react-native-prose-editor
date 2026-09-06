import './helpers/NativeEditorBridgeV2Fixture';
import {
    mockV2Error,
    okRecord,
    errRecord,
    expectNonRetryable,
    catchRejectedNativeRecord,
} from './helpers/NativeEditorBridgeV2Fixture';
import { normalizeNativeEditorV2Result, unwrapNativeEditorV2Result } from '../NativeEditorBridge';

import { NativeEditorV2ErrorBase } from '../NativeEditorBoundaryError';

describe('NativeEditorBridge v2', () => {
    describe('exactly-one result record validation', () => {
        const identity = (value: unknown): unknown => value;

        it('accepts a value-only record (error null or omitted)', () => {
            expect(normalizeNativeEditorV2Result(okRecord('v'), identity)).toEqual({
                ok: true,
                value: 'v',
            });
            expect(normalizeNativeEditorV2Result({ value: 'v' }, identity)).toEqual({
                ok: true,
                value: 'v',
            });
        });

        it('accepts an error-only record (value null or omitted)', () => {
            const error = mockV2Error();
            expect(normalizeNativeEditorV2Result(errRecord(error), identity)).toEqual({
                ok: false,
                error: {
                    domain: 'operation',
                    code: 'OPERATION_INVALID',
                    message: 'operation invalid',
                    requestId: null,
                    operationIndex: null,
                    limit: null,
                    actual: null,
                    details: null,
                },
            });
            expect(normalizeNativeEditorV2Result({ error }, identity)).not.toBeNull();
        });

        it('rejects a record carrying both value and error', () => {
            expect(
                normalizeNativeEditorV2Result({ value: 'v', error: mockV2Error() }, identity)
            ).toBeNull();
        });

        it('rejects a record carrying neither value nor error', () => {
            expect(normalizeNativeEditorV2Result({}, identity)).toBeNull();
            expect(
                normalizeNativeEditorV2Result({ value: null, error: null }, identity)
            ).toBeNull();
        });

        it('rejects non-object records', () => {
            for (const raw of [null, undefined, 42, 'oops', [], true]) {
                expect(normalizeNativeEditorV2Result(raw, identity)).toBeNull();
            }
        });

        it('rejects an error field of the wrong type', () => {
            for (const error of ['oops', 42, [], null]) {
                expect(normalizeNativeEditorV2Result(errRecord(error), identity)).toBeNull();
            }
        });

        it('rejects a value the value normalizer rejects', () => {
            expect(normalizeNativeEditorV2Result(okRecord('nope'), () => null)).toBeNull();
        });

        it('throws the non-retryable class for malformed records on the imperative path', () => {
            const error = catchRejectedNativeRecord(() =>
                unwrapNativeEditorV2Result({ value: 'v', error: mockV2Error() }, (v) => v)
            );
            expectNonRetryable(error, 'FFI_RESULT_INVALID');
            expect((error as NativeEditorV2ErrorBase).domain).toBe('boundary');
        });
    });
});
