import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Keyboard, Platform, StyleSheet, Text, View, type KeyboardEvent } from 'react-native';
import { StatusBar } from 'expo-status-bar';
import * as ImagePicker from 'expo-image-picker';
import { ImageManipulator, SaveFormat } from 'expo-image-manipulator';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';
import {
    createNativeEditorDocumentHandle,
    DEFAULT_EDITOR_IMAGE_LOADING_POLICY,
    NativeRichTextEditor,
    defaultSchema,
    withAtomsSchema,
    withImagesSchema,
    withMentionsSchema,
    type EditorAddons,
    type EditorTheme,
    type ImageRequestContext,
    type LinkRequestContext,
    type MentionQueryChangeEvent,
    type NativeRichTextEditorRef,
} from '@apollohg/react-native-prose-editor';

import { counterCardAtom } from './components/CounterCard';
import { LinkEditorModal } from './components/LinkEditorModal';
import {
    APP_TITLE,
    EDITOR_PLACEHOLDER,
    INITIAL_CONTENT,
    INSERT_COUNTER_ACTION_KEY,
    MENTION_SUGGESTIONS,
    MENTION_TRIGGER,
    TOOLBAR_ITEMS,
} from './content';
import {
    APP_SERIF_FAMILY,
    editorTheme,
    FONT_SIZE,
    LINE_HEIGHT,
    mentionTheme,
    PALETTE,
    RADIUS,
    SPACE,
} from './theme';

const WORD_COUNT_DEBOUNCE_MS = 150;
const PICKED_IMAGE_COMPRESSION = 0.8;
const NEW_COUNTER_TITLE = 'New counter';

const documentSchema = withAtomsSchema(withImagesSchema(withMentionsSchema(defaultSchema)), [
    counterCardAtom,
]);

const editorAtoms = [counterCardAtom];

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
    const editorRef = useRef<NativeRichTextEditorRef>(null);
    const [wordCount, setWordCount] = useState(0);
    const [mentionQuery, setMentionQuery] = useState<string | null>(null);
    const [linkRequest, setLinkRequest] = useState<LinkRequestContext | null>(null);

    const documentHandle = useMemo(
        () =>
            createNativeEditorDocumentHandle({
                schema: documentSchema,
                initialization: { type: 'localHtml', html: INITIAL_CONTENT },
            }),
        []
    );

    useEffect(() => () => documentHandle.destroy(), [documentHandle]);

    const wordCountTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const refreshWordCount = useCallback(() => {
        wordCountTimerRef.current = null;
        const text = editorRef.current?.getTextContent() ?? '';
        setWordCount(countWords(text));
    }, []);

    const handleContentChange = useCallback(() => {
        if (wordCountTimerRef.current != null) {
            return;
        }
        wordCountTimerRef.current = setTimeout(refreshWordCount, WORD_COUNT_DEBOUNCE_MS);
    }, [refreshWordCount]);

    useEffect(() => {
        refreshWordCount();
        return () => {
            if (wordCountTimerRef.current != null) {
                clearTimeout(wordCountTimerRef.current);
                wordCountTimerRef.current = null;
            }
        };
    }, [refreshWordCount]);

    const handleMentionQueryChange = useCallback((event: MentionQueryChangeEvent) => {
        setMentionQuery(event.isActive ? event.query : null);
    }, []);

    const addons = useMemo<EditorAddons>(
        () => ({
            mentions: {
                trigger: MENTION_TRIGGER,
                suggestions: filterMentionSuggestions(mentionQuery),
                theme: mentionTheme,
                onQueryChange: handleMentionQueryChange,
            },
        }),
        [handleMentionQueryChange, mentionQuery]
    );

    const handleToolbarAction = useCallback((key: string) => {
        if (key === INSERT_COUNTER_ACTION_KEY) {
            editorRef.current?.insertContentJson(
                counterCardAtom.buildFragmentJson({ title: NEW_COUNTER_TITLE, count: 0 })
            );
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
        const contentInsets = editorTheme.contentInsets ?? {};
        const bottomEdge = keyboardHeight > 0 ? keyboardHeight : insets.bottom;
        return {
            ...editorTheme,
            contentInsets: { ...contentInsets, bottom: (contentInsets.bottom ?? 0) + bottomEdge },
        };
    }, [insets.bottom, keyboardHeight]);

    return (
        <View style={styles.screen}>
            <View style={[styles.header, { paddingTop: insets.top + SPACE.lg }]}>
                <Text accessibilityRole='header' style={styles.title}>
                    {APP_TITLE}
                </Text>
                <Text style={styles.wordCount}>
                    {wordCount} {wordCount === 1 ? 'word' : 'words'}
                </Text>
            </View>

            <View style={styles.sheet}>
                <NativeRichTextEditor
                    ref={editorRef}
                    documentHandle={documentHandle}
                    atoms={editorAtoms}
                    addons={addons}
                    theme={theme}
                    toolbarItems={TOOLBAR_ITEMS}
                    toolbarPlacement='keyboard'
                    heightBehavior='fixed'
                    placeholder={EDITOR_PLACEHOLDER}
                    accessibilityLabel='Document'
                    accessibilityHint='Formatting is available from the toolbar above the keyboard.'
                    autoCapitalize='sentences'
                    autoCorrect
                    allowImageResizing
                    onContentChange={handleContentChange}
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

function countWords(text: string): number {
    const trimmed = text.trim();
    return trimmed.length === 0 ? 0 : trimmed.split(/\s+/).length;
}

function filterMentionSuggestions(query: string | null) {
    if (query == null) {
        return [];
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
        flexDirection: 'row',
        alignItems: 'baseline',
        justifyContent: 'space-between',
        paddingHorizontal: SPACE.xl,
        paddingBottom: SPACE.lg,
    },
    title: {
        color: PALETTE.paper,
        fontFamily: APP_SERIF_FAMILY,
        fontSize: FONT_SIZE.title,
        lineHeight: LINE_HEIGHT.title,
        fontWeight: '700',
    },
    wordCount: {
        color: PALETTE.mint,
        fontSize: FONT_SIZE.caption,
        lineHeight: LINE_HEIGHT.caption,
        fontVariant: ['tabular-nums'],
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
