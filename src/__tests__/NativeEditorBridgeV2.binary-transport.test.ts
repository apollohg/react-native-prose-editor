import './helpers/NativeEditorBridgeV2Fixture';
import {
    HUGE_U64_DECIMAL,
    MOCK_SNAPSHOT_METADATA,
    MOCK_SNAPSHOT_BYTES,
    okRecord,
    mockNativeModule,
    createHandle,
    expectNonRetryable,
    catchRejectedNativeRecord,
} from './helpers/NativeEditorBridgeV2Fixture';
import { normalizeNativeEditorV2Bytes } from '../NativeEditorBridge';

describe('NativeEditorBridge v2', () => {
    describe('binary transport', () => {
        it('returns an empty exported snapshot as empty bytes', () => {
            const handle = createHandle();
            mockNativeModule.editorV2SnapshotExport.mockReturnValueOnce(
                okRecord({
                    metadataJson: JSON.stringify(MOCK_SNAPSHOT_METADATA),
                    encodedState: new Uint8Array(0),
                })
            );
            const exported = handle.bridge.snapshotExport();
            expect(exported.encodedState).toBeInstanceOf(Uint8Array);
            expect(exported.encodedState.length).toBe(0);
        });

        it('round-trips snapshot bytes byte-for-byte in both directions', () => {
            const handle = createHandle();
            const exported = handle.bridge.snapshotExport();
            expect(exported.metadataJson).toBe(JSON.stringify(MOCK_SNAPSHOT_METADATA));
            expect(exported.encodedState).toBe(MOCK_SNAPSHOT_BYTES);

            const commit = handle.bridge.snapshotRestore(
                MOCK_SNAPSHOT_METADATA,
                exported.encodedState
            );
            const [editorId, metadataJson, encodedState] =
                mockNativeModule.editorV2SnapshotRestore.mock.calls[0];
            expect(editorId).toBe('1');
            expect(JSON.parse(metadataJson)).toEqual(MOCK_SNAPSHOT_METADATA);
            expect(encodedState).toBe(MOCK_SNAPSHOT_BYTES);
            expect(commit.documentRevision).toBe(HUGE_U64_DECIMAL);
        });

        it('rejects JSON number arrays as binary values', () => {
            expect(normalizeNativeEditorV2Bytes([1, 2, 3])).toBeNull();
            const handle = createHandle();
            mockNativeModule.editorV2SnapshotExport.mockReturnValueOnce(
                okRecord({
                    metadataJson: JSON.stringify(MOCK_SNAPSHOT_METADATA),
                    encodedState: [0, 3, 9],
                })
            );
            expectNonRetryable(
                catchRejectedNativeRecord(() => handle.bridge.snapshotExport()),
                'FFI_RESULT_INVALID'
            );
        });
    });
});
