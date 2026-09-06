import './helpers/NativeEditorBridgeV2Fixture';
import {
    HUGE_U64_DECIMAL,
    mockV2Error,
    errRecord,
    expectNonRetryable,
    catchThrown,
} from './helpers/NativeEditorBridgeV2Fixture';
import { normalizeNativeEditorV2Result, unwrapNativeEditorV2Result } from '../NativeEditorBridge';

import {
    NativeEditorEngineBoundaryError,
    NativeEditorDocumentError,
    NativeEditorErrorBase,
    NativeEditorLifecycleError,
    NativeEditorNonRetryableError,
    NativeEditorOperationError,
    NativeEditorSnapshotError,
    NativeEditorTransportError,
    normalizeNativeEditorV2Error,
} from '../NativeEditorBoundaryError';

describe('NativeEditorBridge v2', () => {
    describe('error field, domain, and code validation', () => {
        const identity = (value: unknown): unknown => value;

        it.each([
            ['boundary', NativeEditorEngineBoundaryError],
            ['document', NativeEditorDocumentError],
            ['operation', NativeEditorOperationError],
            ['lifecycle', NativeEditorLifecycleError],
            ['snapshot', NativeEditorSnapshotError],
            ['transport', NativeEditorTransportError],
        ])(
            'throws the structured %s error class for recoverable errors',
            (domain, expectedClass) => {
                const error = catchThrown(() =>
                    unwrapNativeEditorV2Result(
                        errRecord(
                            mockV2Error({
                                domain,
                                requestId: '7',
                                operationIndex: '2',
                                limit: '10',
                                actual: '11',
                                details: { field: 'text' },
                            })
                        ),
                        identity
                    )
                );
                expect(error).toBeInstanceOf(expectedClass);
                expect(error).toBeInstanceOf(NativeEditorErrorBase);
                expect(error).not.toBeInstanceOf(NativeEditorNonRetryableError);
                const typed = error as NativeEditorErrorBase;
                expect(typed.domain).toBe(domain);
                expect(typed.code).toBe('OPERATION_INVALID');
                expect(typed.message).toBe('operation invalid');
                expect(typed.requestId).toBe('7');
                expect(typed.operationIndex).toBe('2');
                expect(typed.limit).toBe('10');
                expect(typed.actual).toBe('11');
                expect(typed.details).toEqual({ field: 'text' });
            }
        );

        it('rejects an unknown domain', () => {
            expect(
                normalizeNativeEditorV2Result(
                    errRecord(mockV2Error({ domain: 'quantum' })),
                    identity
                )
            ).toBeNull();
        });

        it('rejects missing or mistyped required error fields', () => {
            expect(
                normalizeNativeEditorV2Result(
                    errRecord({ code: 'OPERATION_INVALID', message: 'm', domain: 'operation' }),
                    identity
                )
            ).not.toBeNull();
            expect(
                normalizeNativeEditorV2Result(
                    errRecord({ domain: 'operation', message: 'm' }),
                    identity
                )
            ).toBeNull();
            expect(
                normalizeNativeEditorV2Result(
                    errRecord({ domain: 'operation', code: 'X' }),
                    identity
                )
            ).toBeNull();
            expect(
                normalizeNativeEditorV2Result(errRecord(mockV2Error({ code: 42 })), identity)
            ).toBeNull();
            expect(
                normalizeNativeEditorV2Result(errRecord(mockV2Error({ message: null })), identity)
            ).toBeNull();
        });

        it.each(['0', '1', '42', HUGE_U64_DECIMAL])(
            'accepts canonical decimal-string requestId %s of any size',
            (requestId) => {
                const result = normalizeNativeEditorV2Result(
                    errRecord(mockV2Error({ requestId })),
                    identity
                );
                expect(result).not.toBeNull();
                expect(result?.ok).toBe(false);
                if (result && !result.ok) {
                    expect(result.error.requestId).toBe(requestId);
                }
            }
        );

        it.each(['', '01', '-1', '1.0', '+1', ' 1', '1e3', '1 ', '0x10'])(
            'rejects non-canonical requestId %p',
            (requestId) => {
                expect(
                    normalizeNativeEditorV2Result(errRecord(mockV2Error({ requestId })), identity)
                ).toBeNull();
            }
        );

        it('rejects a numeric requestId even when integral', () => {
            expect(
                normalizeNativeEditorV2Result(errRecord(mockV2Error({ requestId: 7 })), identity)
            ).toBeNull();
        });

        it.each(['0', '7', '1024', HUGE_U64_DECIMAL])(
            'accepts canonical decimal string %s for u64 error fields',
            (fieldValue) => {
                const result = normalizeNativeEditorV2Result(
                    errRecord(
                        mockV2Error({
                            operationIndex: fieldValue,
                            limit: fieldValue,
                            actual: fieldValue,
                        })
                    ),
                    identity
                );
                expect(result).not.toBeNull();
            }
        );

        it.each([-1, 1.5, Number.MAX_SAFE_INTEGER + 1, '01', '+1', NaN])(
            'rejects invalid limit field value %p',
            (fieldValue) => {
                expect(
                    normalizeNativeEditorV2Result(
                        errRecord(mockV2Error({ limit: fieldValue })),
                        identity
                    )
                ).toBeNull();
                expect(
                    normalizeNativeEditorV2Result(
                        errRecord(mockV2Error({ operationIndex: fieldValue })),
                        identity
                    )
                ).toBeNull();
                expect(
                    normalizeNativeEditorV2Result(
                        errRecord(mockV2Error({ actual: fieldValue })),
                        identity
                    )
                ).toBeNull();
            }
        );

        it('accepts an object details payload and parses detailsJson', () => {
            const withDetails = normalizeNativeEditorV2Result(
                errRecord(mockV2Error({ details: { field: 'content' } })),
                identity
            );
            expect(withDetails && !withDetails.ok && withDetails.error.details).toEqual({
                field: 'content',
            });
            const withDetailsJson = normalizeNativeEditorV2Result(
                errRecord(mockV2Error({ detailsJson: '{"field":"content"}' })),
                identity
            );
            expect(withDetailsJson && !withDetailsJson.ok && withDetailsJson.error.details).toEqual(
                { field: 'content' }
            );
        });

        it('accepts canonical nested u64 error details from detailsJson', () => {
            const revisionMismatch = normalizeNativeEditorV2Error(
                errRecord(
                    mockV2Error({
                        code: 'REVISION_MISMATCH',
                        detailsJson:
                            '{"expectedRevision":"9007199254740993","actualRevision":"18446744073709551615"}',
                    })
                )
            );
            expect(revisionMismatch?.details).toEqual({
                expectedRevision: '9007199254740993',
                actualRevision: HUGE_U64_DECIMAL,
            });

            const staleGeneration = normalizeNativeEditorV2Error(
                errRecord(
                    mockV2Error({
                        domain: 'transport',
                        code: 'TRANSPORT_STALE_GENERATION',
                        detailsJson:
                            '{"presentedGeneration":"9007199254740993","liveGeneration":null}',
                    })
                )
            );
            expect(staleGeneration?.details).toEqual({
                presentedGeneration: '9007199254740993',
                liveGeneration: null,
            });
        });

        it.each([
            [
                'revision mismatch numeric detailsJson value',
                mockV2Error({
                    code: 'REVISION_MISMATCH',
                    detailsJson:
                        '{"expectedRevision":9007199254740993,"actualRevision":"18446744073709551615"}',
                }),
            ],
            [
                'revision mismatch non-canonical decimal',
                mockV2Error({
                    code: 'REVISION_MISMATCH',
                    detailsJson:
                        '{"expectedRevision":"01","actualRevision":"18446744073709551615"}',
                }),
            ],
            [
                'revision mismatch missing actual revision',
                mockV2Error({
                    code: 'REVISION_MISMATCH',
                    detailsJson: '{"expectedRevision":"9007199254740993"}',
                }),
            ],
            [
                'revision mismatch value above u64 max',
                mockV2Error({
                    code: 'REVISION_MISMATCH',
                    detailsJson:
                        '{"expectedRevision":"18446744073709551615","actualRevision":"18446744073709551616"}',
                }),
            ],
            [
                'stale generation numeric detailsJson value',
                mockV2Error({
                    domain: 'transport',
                    code: 'TRANSPORT_STALE_GENERATION',
                    detailsJson:
                        '{"presentedGeneration":"9007199254740993","liveGeneration":18446744073709551615}',
                }),
            ],
            [
                'stale generation malformed presented value',
                mockV2Error({
                    domain: 'transport',
                    code: 'TRANSPORT_STALE_GENERATION',
                    detailsJson: '{"presentedGeneration":"1e3","liveGeneration":null}',
                }),
            ],
            [
                'stale generation value above u64 max',
                mockV2Error({
                    domain: 'transport',
                    code: 'TRANSPORT_STALE_GENERATION',
                    detailsJson:
                        '{"presentedGeneration":"18446744073709551616","liveGeneration":"18446744073709551615"}',
                }),
            ],
        ])('rejects malformed known nested u64 details: %s', (_label, error) => {
            expect(normalizeNativeEditorV2Error(errRecord(error))).toBeNull();
        });

        it('rejects non-object details payloads', () => {
            for (const details of [[1, 2], 'oops', 42]) {
                expect(
                    normalizeNativeEditorV2Result(errRecord(mockV2Error({ details })), identity)
                ).toBeNull();
            }
            expect(
                normalizeNativeEditorV2Result(
                    errRecord(mockV2Error({ detailsJson: '{invalid' })),
                    identity
                )
            ).toBeNull();
        });

        it('classifies ENGINE_INVARIANT_FAILED as non-retryable', () => {
            const error = catchThrown(() =>
                unwrapNativeEditorV2Result(
                    errRecord(mockV2Error({ code: 'ENGINE_INVARIANT_FAILED' })),
                    identity
                )
            );
            expectNonRetryable(error, 'ENGINE_INVARIANT_FAILED');
            expect(error).not.toBeInstanceOf(NativeEditorOperationError);
        });

        it.each(['ENGINE_DESTROYED', 'ENGINE_DESTROYING'])(
            'classifies lifecycle %s as non-retryable',
            (code) => {
                const error = catchThrown(() =>
                    unwrapNativeEditorV2Result(
                        errRecord(mockV2Error({ domain: 'lifecycle', code })),
                        identity
                    )
                );
                expectNonRetryable(error, code);
                expect((error as NativeEditorErrorBase).domain).toBe('lifecycle');
                expect(error).not.toBeInstanceOf(NativeEditorLifecycleError);
            }
        );

        it('keeps WHOLE_DOCUMENT_REPLACEMENT_CONNECTED a recoverable lifecycle error', () => {
            const error = catchThrown(() =>
                unwrapNativeEditorV2Result(
                    errRecord(
                        mockV2Error({
                            domain: 'lifecycle',
                            code: 'WHOLE_DOCUMENT_REPLACEMENT_CONNECTED',
                        })
                    ),
                    identity
                )
            );
            expect(error).toBeInstanceOf(NativeEditorLifecycleError);
            expect(error).not.toBeInstanceOf(NativeEditorNonRetryableError);
        });
    });
});
