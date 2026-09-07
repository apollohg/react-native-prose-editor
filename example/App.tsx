import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Keyboard, Platform, StyleSheet, Text, View, type KeyboardEvent } from 'react-native';
import { StatusBar } from 'expo-status-bar';
import * as ImagePicker from 'expo-image-picker';
import { ImageManipulator, SaveFormat } from 'expo-image-manipulator';
import { createCodeHighlightingAddon } from '@react-native-rich-text-editor/code-highlighting';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';
import {
    createNativeEditorDocumentHandle,
    DEFAULT_EDITOR_IMAGE_LOADING_POLICY,
    RichTextEditor,
    defaultSchema,
    withAtomsSchema,
    withImagesSchema,
    withMentionsSchema,
    type EditorAddons,
    createMentionsAddon,
    type EditorTheme,
    type ImageRequestContext,
    type LinkRequestContext,
    type MentionQueryChangeEvent,
    type MentionSuggestion,
    type ReadonlyActiveState,
    type RichTextEditorRef,
} from 'react-native-rich-text-editor';

import { counterCardAtom } from './components/CounterCard';
import { LinkEditorModal } from './components/LinkEditorModal';
import {
    APP_TITLE,
    buildToolbarItems,
    EDITOR_PLACEHOLDER,
    INITIAL_DOCUMENT,
    INSERT_COUNTER_ACTION_KEY,
    MENTION_SUGGESTIONS,
    MENTION_TRIGGER,
    TOGGLE_TASK_LIST_ACTION_KEY,
} from './content';
import { TASK_LIST_NODE_NAME, withTaskListSchema } from './taskList';
import { editorTheme, FONT_SIZE, LINE_HEIGHT, mentionTheme, PALETTE, RADIUS, SPACE } from './theme';

const PICKED_IMAGE_COMPRESSION = 0.8;
const NEW_COUNTER_TITLE = 'New counter';

const documentSchema = withAtomsSchema(
    withTaskListSchema(withImagesSchema(withMentionsSchema(defaultSchema))),
    [counterCardAtom]
);

const editorAtoms = [counterCardAtom];

const codeHighlightingAddon = createCodeHighlightingAddon({ theme: 'InspiredGitHub' });

const NO_MENTION_SUGGESTIONS: readonly MentionSuggestion[] = [];

export default function App() {
    return (
        <SafeAreaProvider>
            <StatusBar style='light' />
            <EditorScreen />
        </SafeAreaProvider>
    );
}

function EditorScreen() {
    const insets = useSafeAreaInsets();
    const keyboardHeight = useKeyboardHeight();
    const editorRef = useRef<RichTextEditorRef>(null);
    const [mentionSuggestions, setMentionSuggestions] =
        useState<readonly MentionSuggestion[]>(NO_MENTION_SUGGESTIONS);
    const [linkRequest, setLinkRequest] = useState<LinkRequestContext | null>(null);
    const [taskListActive, setTaskListActive] = useState(false);
    const [taskListAvailable, setTaskListAvailable] = useState(false);

    const documentHandle = useMemo(
        () =>
            createNativeEditorDocumentHandle({
                schema: documentSchema,
                initialization: { type: 'localJson', json: INITIAL_DOCUMENT },
            }),
        []
    );

    useEffect(() => () => documentHandle.destroy(), [documentHandle]);

    /** Keeps the previous list when the filter result is unchanged, so the addons prop is stable across keystrokes. */
    const handleMentionQueryChange = useCallback((event: MentionQueryChangeEvent) => {
        const next = filterMentionSuggestions(event.isActive ? event.query : null);
        setMentionSuggestions((current) => (sameSuggestions(current, next) ? current : next));
    }, []);

    const addons = useMemo<EditorAddons>(
        () => [
            codeHighlightingAddon,
            createMentionsAddon({
                trigger: MENTION_TRIGGER,
                suggestions: mentionSuggestions,
                theme: mentionTheme,
                onQueryChange: handleMentionQueryChange,
            }),
        ],
        [handleMentionQueryChange, mentionSuggestions]
    );

    const handleActiveStateChange = useCallback((state: ReadonlyActiveState) => {
        setTaskListActive(state.nodes[TASK_LIST_NODE_NAME] === true);
        setTaskListAvailable(state.commands.wrapTaskList === true);
    }, []);

    const toolbarItems = useMemo(
        () => buildToolbarItems({ taskListActive, taskListAvailable }),
        [taskListActive, taskListAvailable]
    );

    const handleToolbarAction = useCallback((key: string) => {
        switch (key) {
            case INSERT_COUNTER_ACTION_KEY:
                editorRef.current?.insertContentJson(
                    counterCardAtom.buildFragmentJson({ title: NEW_COUNTER_TITLE, count: 0 })
                );
                break;
            case TOGGLE_TASK_LIST_ACTION_KEY:
                editorRef.current?.toggleList(TASK_LIST_NODE_NAME);
                break;
        }
    }, []);

    const handleRequestImage = useCallback((context: ImageRequestContext) => {
        void pickImageUri().then((uri) => {
            if (uri != null) {
                context.insertImage(uri);
            }
        });
    }, []);

    const closeLinkRequest = useCallback(() => setLinkRequest(null), []);

    /** The sheet runs to the screen edge, so the keyboard becomes a content inset instead of a layout cut. */
    const theme = useMemo<EditorTheme>(() => {
        const content = editorTheme.content;
        const bottomEdge = keyboardHeight > 0 ? keyboardHeight : insets.bottom;
        return {
            ...editorTheme,
            content: { ...content, paddingBottom: content.paddingBottom + bottomEdge },
        };
    }, [insets.bottom, keyboardHeight]);

    return (
        <View style={styles.screen}>
            <View style={[styles.header, { paddingTop: insets.top + SPACE.lg }]}>
                <Text accessibilityRole='header' style={styles.title}>
                    {APP_TITLE}
                </Text>
            </View>

            <View style={styles.sheet}>
                <RichTextEditor
                    ref={editorRef}
                    documentHandle={documentHandle}
                    atoms={editorAtoms}
                    addons={addons}
                    theme={theme}
                    toolbarItems={toolbarItems}
                    toolbarPlacement='keyboard'
                    heightBehavior='fixed'
                    placeholder={EDITOR_PLACEHOLDER}
                    accessibilityLabel='Document'
                    accessibilityHint='Formatting is available from the toolbar above the keyboard.'
                    autoCapitalize='sentences'
                    autoCorrect
                    allowImageResizing
                    onActiveStateChange={handleActiveStateChange}
                    onToolbarAction={handleToolbarAction}
                    onRequestLink={setLinkRequest}
                    onRequestImage={handleRequestImage}
                    containerStyle={styles.editorContainer}
                    style={styles.editor}
                />
            </View>

            <LinkEditorModal request={linkRequest} onClose={closeLinkRequest} />
        </View>
    );
}

/**
 * Keyboard height on iOS, where the window keeps its size and the editor
 * scrolls behind the keyboard. Android resizes the window itself, so 0.
 */
function useKeyboardHeight(): number {
    const [height, setHeight] = useState(0);

    useEffect(() => {
        if (Platform.OS !== 'ios') {
            return;
        }
        const show = Keyboard.addListener('keyboardWillShow', (event: KeyboardEvent) =>
            setHeight(event.endCoordinates.height)
        );
        const hide = Keyboard.addListener('keyboardWillHide', () => setHeight(0));
        return () => {
            show.remove();
            hide.remove();
        };
    }, []);

    return height;
}

function filterMentionSuggestions(query: string | null): readonly MentionSuggestion[] {
    if (query == null) {
        return NO_MENTION_SUGGESTIONS;
    }
    const needle = query.trim().toLowerCase();
    if (needle.length === 0) {
        return MENTION_SUGGESTIONS;
    }
    return MENTION_SUGGESTIONS.filter(
        (suggestion) =>
            suggestion.title.toLowerCase().includes(needle) ||
            (suggestion.label ?? '').toLowerCase().includes(needle)
    );
}

function sameSuggestions(
    a: readonly MentionSuggestion[],
    b: readonly MentionSuggestion[]
): boolean {
    return a.length === b.length && a.every((suggestion, index) => suggestion.key === b[index].key);
}

/** Opens the photo library and downsizes the pick to the editor's decode limit. */
async function pickImageUri(): Promise<string | null> {
    const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
    if (!permission.granted) {
        return null;
    }

    const result = await ImagePicker.launchImageLibraryAsync({ quality: 1 });
    if (result.canceled || result.assets.length === 0) {
        return null;
    }

    const asset = result.assets[0];
    const decodeLimit = DEFAULT_EDITOR_IMAGE_LOADING_POLICY.maxDecodeDimensionPx;
    const context = ImageManipulator.manipulate(asset.uri);
    if (asset.width > decodeLimit) {
        context.resize({ width: decodeLimit });
    }
    const rendered = await context.renderAsync();
    const saved = await rendered.saveAsync({
        format: SaveFormat.JPEG,
        compress: PICKED_IMAGE_COMPRESSION,
    });
    return saved.uri;
}

const styles = StyleSheet.create({
    screen: {
        flex: 1,
        backgroundColor: PALETTE.spruceDeep,
    },
    header: {
        paddingHorizontal: SPACE.xl,
        paddingBottom: SPACE.lg,
    },
    title: {
        color: PALETTE.paper,
        fontSize: FONT_SIZE.title,
        lineHeight: LINE_HEIGHT.title,
        fontWeight: '700',
    },
    sheet: {
        flex: 1,
        overflow: 'hidden',
        backgroundColor: PALETTE.paper,
        borderTopLeftRadius: RADIUS.sheet,
        borderTopRightRadius: RADIUS.sheet,
    },
    editorContainer: {
        flex: 1,
    },
    editor: {
        flex: 1,
    },
});
