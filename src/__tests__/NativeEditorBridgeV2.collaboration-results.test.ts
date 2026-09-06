import './helpers/NativeEditorBridgeV2Fixture';
import {
    mockNativeModule,
    createHandle,
    compileTypeScriptContractFixture,
} from './helpers/NativeEditorBridgeV2Fixture';
import {
    createNativeEditorLocalAwarenessSelection,
    type NativeEditorLocalAwarenessIntent,
} from '../NativeEditorBridge';

describe('NativeEditorBridge v2', () => {
    describe('collaboration results', () => {
        it('creates a frozen local-awareness selection and serializes its tagged wire intent', () => {
            const handle = createHandle();
            const selection = createNativeEditorLocalAwarenessSelection(2, 5);
            const intent: NativeEditorLocalAwarenessIntent = {
                state: { user: { name: 'Alice' } },
                focused: true,
                selection,
            };

            expect(selection).toEqual({ anchor: 2, head: 5 });
            expect(Object.isFrozen(selection)).toBe(true);
            expect(() => Object.assign(selection, { anchor: 3 })).toThrow();

            handle.bridge.setLocalAwareness(intent);
            handle.bridge.setLocalAwareness(null);
            const calls = mockNativeModule.editorV2CollaborationSetAwareness.mock.calls;
            expect(JSON.parse(calls[0][1])).toEqual({
                selection: { type: 'text', anchor: 2, head: 5 },
                state: { user: { name: 'Alice' } },
                focused: true,
            });
            expect(intent.selection).toEqual({ anchor: 2, head: 5 });
            expect(calls[1][1]).toBe('null');
        });

        it('rejects literal, cloned, cast, and proxied selections before native invocation', () => {
            const handle = createHandle();
            const factorySelection = createNativeEditorLocalAwarenessSelection(2, 5);
            let accessorRead = false;
            const transparentProxy = new Proxy(factorySelection, {});
            const accessorProxy = new Proxy(factorySelection, {
                get: () => {
                    accessorRead = true;
                    throw new Error('selection accessor must not be read');
                },
            });
            const accessorSelection: Record<string, unknown> = { type: 'text', head: 5 };
            Object.defineProperty(accessorSelection, 'anchor', {
                enumerable: true,
                get: () => {
                    throw new Error('selection accessor must not be read');
                },
            });
            const invalidSelections: unknown[] = [
                { anchor: 2, head: 5 },
                { ...factorySelection },
                transparentProxy,
                accessorProxy,
                { type: 'text', anchor: 2, head: 5 },
                { type: 'node', pos: 2 },
                { type: 'all' },
                { anchor: 2, head: 5, pos: 2 },
                { anchor: 2, head: 5, anchorScalar: 2 },
                { anchor: 2, head: 5, headScalar: 5 },
                { anchor: 2, head: 5, posScalar: 2 },
                { anchor: 2, head: 5, extra: true },
                { anchor: -1, head: 5 },
                { anchor: 2, head: 0x1_0000_0000 },
                { anchor: 2.5, head: 5 },
                Object.assign(Object.create({ anchor: 2 }), { type: 'text', head: 5 }),
                accessorSelection,
            ];

            for (const selection of invalidSelections) {
                expect(() =>
                    handle.bridge.setLocalAwareness({
                        state: { user: { name: 'Alice' } },
                        focused: true,
                        selection,
                    } as unknown as NativeEditorLocalAwarenessIntent)
                ).toThrow('invalid local awareness intent');
            }
            expect(accessorRead).toBe(false);
            expect(mockNativeModule.editorV2CollaborationSetAwareness).not.toHaveBeenCalled();
        });

        it('rejects invalid u32 factory coordinates before an intent can reach native code', () => {
            const invalidCoordinates = [
                -1,
                1.5,
                Number.NaN,
                Number.POSITIVE_INFINITY,
                0x1_0000_0000,
            ];

            for (const coordinate of invalidCoordinates) {
                expect(() => createNativeEditorLocalAwarenessSelection(coordinate, 1)).toThrow(
                    'invalid local awareness intent'
                );
                expect(() => createNativeEditorLocalAwarenessSelection(1, coordinate)).toThrow(
                    'invalid local awareness intent'
                );
            }
            expect(mockNativeModule.editorV2CollaborationSetAwareness).not.toHaveBeenCalled();
        });

        it('rejects raw and recursively cursor-authored awareness before invoking native code', () => {
            const handle = createHandle();
            const rawState = { user: { name: 'Alice' } };
            const cursorIntent = {
                state: {
                    user: { name: 'Alice' },
                    metadata: [{ cursor: { sticky: 'caller-authored' } }],
                },
                focused: false,
            };

            expect(() =>
                handle.bridge.setLocalAwareness(
                    rawState as unknown as NativeEditorLocalAwarenessIntent
                )
            ).toThrow('invalid local awareness intent');
            expect(() =>
                handle.bridge.setLocalAwareness(cursorIntent as NativeEditorLocalAwarenessIntent)
            ).toThrow('reserved cursor key');
            expect(cursorIntent).toEqual({
                state: {
                    user: { name: 'Alice' },
                    metadata: [{ cursor: { sticky: 'caller-authored' } }],
                },
                focused: false,
            });
            expect(mockNativeModule.editorV2CollaborationSetAwareness).not.toHaveBeenCalled();
        });

        it('exports only the factory-created opaque local awareness selection contract', () => {
            const diagnostics = compileTypeScriptContractFixture(`
                import {
                    createNativeEditorLocalAwarenessSelection,
                    type NativeEditorLocalAwarenessIntent,
                } from '../index';

                type LocalAwarenessSelection = NonNullable<
                    NativeEditorLocalAwarenessIntent['selection']
                >;

                const selection: LocalAwarenessSelection =
                    createNativeEditorLocalAwarenessSelection(2, 5);
                const intent: NativeEditorLocalAwarenessIntent = {
                    state: { user: { name: 'Alice' } },
                    focused: true,
                    selection,
                };

                // @ts-expect-error only the factory can create caller-awareness selections.
                const literal: LocalAwarenessSelection = { anchor: 2, head: 5 };
                // @ts-expect-error the Rust discriminator is bridge-owned wire data.
                const tagged: LocalAwarenessSelection = { type: 'text', anchor: 2, head: 5 };
                // @ts-expect-error clones do not carry the opaque type brand.
                const cloned: LocalAwarenessSelection = { ...selection };
                void intent;
                void literal;
                void tagged;
                void cloned;
            `);

            expect(diagnostics).toBe('');
        });
    });
});
