import './helpers/NativeEditorBridgeV2Fixture';
import { mockNativeModule, createHandle } from './helpers/NativeEditorBridgeV2Fixture';

describe('NativeEditorBridge v2', () => {
    describe('v2 surface availability', () => {
        it('fails clearly when the native module does not expose the v2 surface', () => {
            delete mockNativeModule.editorV2Create;
            expect(() => createHandle()).toThrow(/editorV2Create/);
        });
    });
});
