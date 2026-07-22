import React from 'react';
import { StyleSheet, Text, View } from 'react-native';
import {
    NativeRichTextEditor,
    type DocumentJSON,
    type EditorAddons,
    type NativeEditorDocumentHandle,
    type NativeRichTextEditorRef,
} from '@apollohg/react-native-prose-editor';
import type { ExampleThemePreset } from '../themePresets';
import { sharedStyles } from '../sharedStyles';

type EditorDemoCardProps = {
    editorRef: React.RefObject<NativeRichTextEditorRef | null>;
    /** The shared document session; the same handle feeds the collaboration controller. */
    documentHandle: NativeEditorDocumentHandle;
    /** Revision signal from the collaboration controller (remote commits, promotions). */
    documentRevision?: string | null;
    /** Pinged after each local engine mutation so collaboration can flush outbound frames. */
    onLocalDocumentCommit?: () => void;
    theme: React.ComponentProps<typeof NativeRichTextEditor>['theme'];
    addons?: EditorAddons;
    onContentChange: (html: string) => void;
    onContentChangeJSON: (json: DocumentJSON) => void;
    remoteSelections?: React.ComponentProps<typeof NativeRichTextEditor>['remoteSelections'];
    appChrome: ExampleThemePreset['appChrome'];
};

export function EditorDemoCard({
    editorRef,
    documentHandle,
    documentRevision,
    onLocalDocumentCommit,
    theme,
    addons,
    onContentChange,
    onContentChangeJSON,
    remoteSelections,
    appChrome,
}: EditorDemoCardProps) {
    return (
        <View style={[styles.card, { backgroundColor: appChrome.cardSecondaryBackgroundColor }]}>
            <Text style={[sharedStyles.sectionLabel, { color: appChrome.sectionLabelColor }]}>
                Editor
            </Text>

            <NativeRichTextEditor
                ref={editorRef}
                documentHandle={documentHandle}
                documentRevision={documentRevision}
                onLocalDocumentCommit={onLocalDocumentCommit}
                theme={theme}
                addons={addons}
                placeholder='Start typing...'
                onContentChange={onContentChange}
                onContentChangeJSON={onContentChangeJSON}
                remoteSelections={remoteSelections}
                style={styles.editor}
            />
        </View>
    );
}

const styles = StyleSheet.create({
    card: {
        borderRadius: 24,
        padding: 14,
        gap: 10,
    },
    editor: {
        borderRadius: 16,
        minHeight: 200,
        maxHeight: 300,
    },
});
