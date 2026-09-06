import './helpers/NativeEditorBridgeV2Fixture';

import { normalizeNativeEditorV2Unit } from '../NativeEditorBridge';

describe('NativeEditorBridge v2', () => {
    describe('unit results', () => {
        it('accepts only the literal true success marker', () => {
            expect(normalizeNativeEditorV2Unit(true)).toBe(true);
            expect(normalizeNativeEditorV2Unit(false)).toBeNull();
            expect(normalizeNativeEditorV2Unit('true')).toBeNull();
            expect(normalizeNativeEditorV2Unit(1)).toBeNull();
            expect(normalizeNativeEditorV2Unit(null)).toBeNull();
        });
    });
});
