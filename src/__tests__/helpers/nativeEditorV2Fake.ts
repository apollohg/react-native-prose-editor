export {
    V2_FAKE_STEP1_FRAME,
    V2_FAKE_STEP2_FRAME,
    V2_FAKE_STEP2_INVALID_FRAGMENT_FRAME,
    V2_FAKE_UPDATE_FRAME,
    V2_FAKE_AWARENESS_FRAME,
    V2_FAKE_MALFORMED_FRAME,
    V2_FAKE_INCOMPATIBLE_FRAME,
} from './nativeEditorV2FakeRecords';
export { fakeHtmlForDoc, fakeDocForHtml, fakeDocForText } from './nativeEditorV2FakeDocument';
export {
    type FakeProtocolAdapterResolution,
    type FakeV2SessionHandle,
    type FakeAwarenessBroadcastFailureCode,
    type FakeNativeEditorV2Runtime,
} from './nativeEditorV2FakeTypes';
export { createFakeNativeEditorV2Runtime } from './createFakeNativeEditorV2Runtime';
