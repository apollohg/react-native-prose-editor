import './helpers/NativeEditorBridgeV2Fixture';
import {
    MOCK_V2_STATE,
    MOCK_V2_TRANSACTION,
    HUGE_U64_DECIMAL,
    ONE_OVER_U64_DECIMAL,
    okRecord,
    mockNativeModule,
    createHandle,
    expectNonRetryable,
    catchRejectedNativeRecord,
} from './helpers/NativeEditorBridgeV2Fixture';
import { normalizeNativeEditorV2DecimalId, requireNativeEditorV2U32 } from '../NativeEditorBridge';

describe('NativeEditorBridge v2', () => {
    describe('decimal identifiers and unsafe integers', () => {
        it('normalizes canonical decimal strings of any size verbatim', () => {
            expect(normalizeNativeEditorV2DecimalId('0')).toBe('0');
            expect(normalizeNativeEditorV2DecimalId('42')).toBe('42');
            expect(normalizeNativeEditorV2DecimalId(HUGE_U64_DECIMAL)).toBe(HUGE_U64_DECIMAL);
        });

        it('accepts u64::MAX and rejects larger decimal strings without Number()', () => {
            expect(normalizeNativeEditorV2DecimalId(HUGE_U64_DECIMAL)).toBe(HUGE_U64_DECIMAL);
            expect(normalizeNativeEditorV2DecimalId(ONE_OVER_U64_DECIMAL)).toBeNull();
            expect(normalizeNativeEditorV2DecimalId('9'.repeat(256))).toBeNull();
        });

        it.each([0, 42, Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER + 1])(
            'rejects numeric compatibility value %p even when safely representable',
            (value) => {
                expect(normalizeNativeEditorV2DecimalId(value)).toBeNull();
            }
        );

        it.each(['', '01', '-1', '1.0', '+1', ' 1', '1 ', '1e3', '1E3'])(
            'rejects every non-canonical decimal string %p',
            (value) => {
                expect(normalizeNativeEditorV2DecimalId(value)).toBeNull();
            }
        );

        it.each([Number.MAX_SAFE_INTEGER + 1, -1, 1.5, NaN, Infinity])(
            'rejects unsafe or non-integer number %p',
            (value) => {
                expect(normalizeNativeEditorV2DecimalId(value)).toBeNull();
            }
        );

        it('keeps huge decimal-string revisions verbatim in state results', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        ...MOCK_V2_STATE,
                        documentRevision: HUGE_U64_DECIMAL,
                        stateRevision: HUGE_U64_DECIMAL,
                    })
                )
            );
            const state = handle.bridge.getState();
            expect(state.documentRevision).toBe(HUGE_U64_DECIMAL);
            expect(state.stateRevision).toBe(HUGE_U64_DECIMAL);
            expect(typeof state.documentRevision).toBe('string');
        });

        it('rejects numeric revision compatibility values even below the JavaScript safe limit', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockReturnValueOnce(
                okRecord(JSON.stringify({ ...MOCK_V2_STATE, documentRevision: 4 }))
            );
            expectNonRetryable(
                catchRejectedNativeRecord(() => handle.bridge.getState()),
                'FFI_RESULT_INVALID'
            );
        });

        it.each([
            [0, 0],
            [1, 1],
            [0xffff_ffff, 0xffff_ffff],
        ])('accepts exact u32 value %p for %s', (value, expected) => {
            expect(requireNativeEditorV2U32(value, 'scalar')).toBe(expected);
        });

        it.each([-1, 1.5, NaN, Infinity, 0x1_0000_0000])(
            'rejects non-exact or out-of-range u32 value %p',
            (value) => {
                expect(() => requireNativeEditorV2U32(value, 'scalar')).toThrow(
                    'invalid u32 scalar'
                );
            }
        );

        it('rejects an unsafe integer revision in a state result', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        ...MOCK_V2_STATE,
                        documentRevision: Number.MAX_SAFE_INTEGER + 1,
                    })
                )
            );
            expectNonRetryable(
                catchRejectedNativeRecord(() => handle.bridge.getState()),
                'FFI_RESULT_INVALID'
            );
        });

        it('rejects a leading-zero revision string in a state result', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockReturnValueOnce(
                okRecord(JSON.stringify({ ...MOCK_V2_STATE, documentRevision: '04' }))
            );
            expectNonRetryable(
                catchRejectedNativeRecord(() => handle.bridge.getState()),
                'FFI_RESULT_INVALID'
            );
        });

        it('normalizes transaction outcome revisions to decimal strings', () => {
            const handle = createHandle();
            const outcome = handle.bridge.applyInput({ baseDocumentRevision: '4', text: 'hi' });
            expect(outcome).toEqual({
                type: 'transaction',
                changed: true,
                documentRevision: '5',
                stateRevision: '3',
                canUndo: true,
                canRedo: false,
            });
        });

        it('rejects an unsafe integer revision in a transaction outcome', () => {
            const handle = createHandle();
            mockNativeModule.editorV2ApplyInput.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        ...MOCK_V2_TRANSACTION,
                        stateRevision: Number.MAX_SAFE_INTEGER + 1,
                    })
                )
            );
            expectNonRetryable(
                catchRejectedNativeRecord(() =>
                    handle.bridge.applyInput({ baseDocumentRevision: '4', text: 'x' })
                ),
                'FFI_RESULT_INVALID'
            );
        });
    });
});
