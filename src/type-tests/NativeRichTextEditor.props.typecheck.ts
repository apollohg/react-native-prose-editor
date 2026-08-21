import type { NativeEditorDocumentHandle } from '../NativeEditorBridge';
import type {
    ExternalTextCompositionEndEvent,
    NativeRichTextEditorFocusPreservingRef,
    NativeRichTextEditorRef,
    ReadonlyActiveState,
} from '../index';
import type { NativeRichTextEditorProps } from '../NativeRichTextEditor';

declare const documentHandle: NativeEditorDocumentHandle;

// Compile-only public API contract. `npm run typecheck` must fail if a
// handle-creation field ever returns to NativeRichTextEditorProps.
const removedComponentProps: readonly NativeRichTextEditorProps[] = [
    {
        documentHandle,
        // @ts-expect-error schema belongs to handle creation
        schema: undefined,
    },
    {
        documentHandle,
        // @ts-expect-error resource limits belong to handle creation
        resourceLimits: undefined,
    },
    {
        documentHandle,
        // @ts-expect-error base64 policy belongs to handle creation
        allowBase64Images: false,
    },
    {
        documentHandle,
        // @ts-expect-error maximum length belongs to handle creation
        maxLength: 1,
    },
    {
        documentHandle,
        // @ts-expect-error engine read-only belongs to handle creation
        readOnly: true,
    },
    {
        documentHandle,
        // @ts-expect-error input filtering belongs to handle creation
        inputFilter: '[a-z]',
    },
    {
        documentHandle,
        // @ts-expect-error fragment selection belongs to handle creation
        fragmentName: 'prosemirror',
    },
    {
        documentHandle,
        // @ts-expect-error initial HTML belongs to handle creation
        initialContent: '<p>legacy</p>',
    },
    {
        documentHandle,
        // @ts-expect-error initial JSON belongs to handle creation
        initialJSON: { type: 'doc', content: [] },
    },
];

void removedComponentProps;

declare const readonlyActiveState: ReadonlyActiveState;
void readonlyActiveState;

const readonlyActiveStateCallback: NonNullable<NativeRichTextEditorProps['onActiveStateChange']> = (
    state
) => {
    // @ts-expect-error render snapshots must remain recursively immutable for consumers
    state.marks.bold = false;
};

void readonlyActiveStateCallback;

declare const focusPreservingRef: NativeRichTextEditorFocusPreservingRef;

const focusPreservingProps: readonly NativeRichTextEditorProps[] = [
    { documentHandle, focusPreservingRefs: focusPreservingRef },
    { documentHandle, focusPreservingRefs: [focusPreservingRef] as const },
];

void focusPreservingProps;

declare const editorRef: NativeRichTextEditorRef;

async function driveExternalComposition(): Promise<void> {
    if (!editorRef.supportsExternalTextComposition()) return;
    const session = await editorRef.beginExternalTextComposition({
        onEnd(event: ExternalTextCompositionEndEvent) {
            void event.outcome;
        },
    });
    await session.update('on arrival');
    await session.commit('O/A');
    await session.cancel();
}

void driveExternalComposition;
