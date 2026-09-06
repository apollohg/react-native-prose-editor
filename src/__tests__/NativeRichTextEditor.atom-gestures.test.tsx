import './helpers/NativeRichTextEditorFixture';
import {
    atomBlock,
    counterAtomDefinition,
    installAtomRenderSource,
} from './helpers/NativeRichTextEditorFixture';
import { act, render } from '@testing-library/react-native';
import { Platform, StyleSheet } from 'react-native';
import Pressability from 'react-native/Libraries/Pressability/Pressability';
import { NativeRichTextEditor } from '../NativeRichTextEditor';
import { createNativeEditorDocumentHandle } from '../NativeEditorBridge';
import { withAtomsSchema } from '../atoms';
import { tiptapCompatibleSchema } from '../schemas';

test('Android atom presses survive finger movement after native layout and scrolling', () => {
    const platform = jest.replaceProperty(Platform, 'OS', 'android');
    const { definition } = counterAtomDefinition();
    const handle = createNativeEditorDocumentHandle({
        schema: withAtomsSchema(tiptapCompatibleSchema, [definition]),
        initialization: {
            type: 'localJson',
            json: { type: 'doc', content: [{ type: 'counterCard', attrs: { title: 'a' } }] },
        },
    });
    installAtomRenderSource(() => ({
        renderBlocks: atomBlock('counterCard', 1, 'counter'),
        renderPatch: null,
    }));
    const screen = render(<NativeRichTextEditor documentHandle={handle} atoms={[definition]} />);
    const nativeView = screen.getByTestId('native-editor-view');
    try {
        for (const scrollY of [0, 120]) {
            const contentInset = 18;
            act(() => {
                nativeView.props.onAtomLayout({
                    nativeEvent: {
                        editorId: handle.editorId,
                        width: 280,
                        positions: [
                            {
                                key: 'counter',
                                x: 12,
                                y: 400,
                                width: 280,
                                height: 100,
                                hostX: 12,
                                hostY: 400 + contentInset - scrollY,
                            },
                        ],
                        viewport: { y: scrollY, height: 500 },
                    },
                });
            });
            const style = StyleSheet.flatten(
                screen.UNSAFE_getByProps({ nativeID: 'prose-atom:counter' }).props.style
            );
            let count = 0;
            const onPressIn = jest.fn();
            const pressability = new Pressability({ onPressIn, onPress: () => count++ });
            const handlers = pressability.getEventHandlers();
            const rootY = 100;
            const buttonX = 220;
            const buttonY = 28;
            const responder = {
                // Fabric measure uses the React shadow layout, before native reparenting.
                measure: (callback: (...values: number[]) => void) =>
                    callback(
                        buttonX,
                        buttonY,
                        44,
                        44,
                        Number(style.left) + buttonX,
                        rootY + Number(style.top) + buttonY
                    ),
            };
            const event = (movement: number) => ({
                currentTarget: responder,
                persist() {},
                nativeEvent: {
                    pageX: 12 + buttonX + 22 + movement,
                    pageY: rootY + 400 + contentInset - scrollY + buttonY + 22 + movement,
                    locationX: 22 + movement,
                    locationY: 22 + movement,
                    timestamp: 100 + movement,
                },
            });
            handlers.onResponderGrant(event(0) as never);
            handlers.onResponderMove(event(1) as never);
            handlers.onResponderRelease(event(1) as never);
            pressability.reset();
            expect(onPressIn).toHaveBeenCalledTimes(1);
            expect(count).toBe(1);
        }
    } finally {
        screen.unmount();
        handle.destroy();
        platform.restore();
    }
});
