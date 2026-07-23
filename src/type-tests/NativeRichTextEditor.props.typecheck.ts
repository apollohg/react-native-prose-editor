import type { NativeEditorDocumentHandle } from '../NativeEditorBridge';
import type { ReadonlyActiveState } from '../index';
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

const readonlyActiveStateCallback: NonNullable<NativeRichTextEditorProps['onActiveStateChange']> =
    (state) => {
        // @ts-expect-error render snapshots must remain recursively immutable for consumers
        state.marks.bold = false;
    };

void readonlyActiveStateCallback;
