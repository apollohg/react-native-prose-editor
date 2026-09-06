import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeModule,
    mockResolveDocumentDescriptor,
    HANDLE_OWNED_ARTICLE_SCHEMA,
    V2_INITIAL_DOC,
    V2_DOC_B,
    V2_DOC_C,
    createV2LocalHandle,
} from './helpers/NativeRichTextEditorFixture';
import { createRef } from 'react';

import { render, act } from '@testing-library/react-native';
import {
    NativeRichTextEditor,
    type NativeRichTextEditorProps,
    type NativeRichTextEditorRef,
} from '../NativeRichTextEditor';
import { createNativeEditorDocumentHandle } from '../NativeEditorBridge';

import { fakeDocForText } from './helpers/nativeEditorV2Fake';

import { type SchemaDefinition } from '../schemas';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('accepts only handle-bound view props and never creates a session on mount', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);

        // The @ts-expect-error assertions are the public hard-cutover
        // contract. Each field belongs to NativeEditorCreateConfig, never
        // to the mounted view.
        const removedComponentProps: readonly NativeRichTextEditorProps[] = [
            {
                documentHandle: handle,
                // @ts-expect-error schema belongs to handle creation
                schema: undefined,
            },
            {
                documentHandle: handle,
                // @ts-expect-error resource limits belong to handle creation
                resourceLimits: undefined,
            },
            {
                documentHandle: handle,
                // @ts-expect-error base64 policy belongs to handle creation
                allowBase64Images: false,
            },
            {
                documentHandle: handle,
                // @ts-expect-error maximum length belongs to handle creation
                maxLength: 1,
            },
            {
                documentHandle: handle,
                // @ts-expect-error engine read-only belongs to handle creation
                readOnly: true,
            },
            {
                documentHandle: handle,
                // @ts-expect-error input filtering belongs to handle creation
                inputFilter: '[a-z]',
            },
            {
                documentHandle: handle,
                // @ts-expect-error fragment selection belongs to handle creation
                fragmentName: 'prosemirror',
            },
            {
                documentHandle: handle,
                // @ts-expect-error initial HTML belongs to handle creation
                initialContent: '<p>legacy</p>',
            },
            {
                documentHandle: handle,
                // @ts-expect-error initial JSON belongs to handle creation
                initialJSON: V2_INITIAL_DOC,
            },
        ];
        expect(removedComponentProps).toHaveLength(9);

        mockNativeModule.editorV2Create.mockClear();
        mockResolveDocumentDescriptor.mockClear();
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} editable={false} />
        );

        expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        expect(mockResolveDocumentDescriptor).not.toHaveBeenCalled();
        expect(getByTestId('native-editor-view').props.editorId).toBe(handle.editorId);
        handle.destroy();
    });

    it('uses custom-root metadata owned by the handle for clear and image fragments', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: HANDLE_OWNED_ARTICLE_SCHEMA,
            initialization: { type: 'localEmpty' },
        });
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);

        act(() => ref.current!.clearContent());
        const clearRequest = JSON.parse(
            mockNativeModule.editorV2ReplaceDocument.mock.calls[
                mockNativeModule.editorV2ReplaceDocument.mock.calls.length - 1
            ][1] as string
        ) as Record<string, unknown>;
        expect(clearRequest.setJson).toEqual({
            type: 'article',
            content: [{ type: 'title' }],
        });

        act(() => ref.current!.insertImage('https://example.test/image.png'));
        const imageRequest = JSON.parse(
            mockNativeModule.editorV2ApplyCommand.mock.calls[
                mockNativeModule.editorV2ApplyCommand.mock.calls.length - 1
            ][1] as string
        ) as { command: Record<string, unknown> };
        expect(imageRequest.command).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'article',
                content: [{ type: 'image', attrs: { src: 'https://example.test/image.png' } }],
            },
        });

        handle.destroy();
    });

    it('normalizes an empty controlled custom-root document from its handle descriptor', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: HANDLE_OWNED_ARTICLE_SCHEMA,
            initialization: { type: 'localEmpty' },
        });
        const { rerender } = render(<NativeRichTextEditor documentHandle={handle} />);
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                documentHandle={handle}
                valueJSON={{ type: 'article', content: [] }}
            />
        );

        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        const request = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(request.setJson).toEqual({
            type: 'article',
            content: [{ type: 'title' }],
        });

        handle.destroy();
    });

    it('retains immutable custom-root defaults after the caller mutates its schema', () => {
        const mutableCardDefault = {
            appearance: { tone: 'blue' },
            labels: ['initial'],
        };
        const schema: SchemaDefinition = {
            nodes: [
                { name: 'article', content: 'card', role: 'doc' },
                {
                    name: 'card',
                    content: '',
                    group: 'block',
                    role: 'block',
                    attrs: { config: { default: mutableCardDefault } },
                },
                { name: 'text', content: '', group: 'inline', role: 'text' },
            ],
            marks: [],
        };
        const handle = createNativeEditorDocumentHandle({
            schema,
            initialization: { type: 'localEmpty' },
        });
        mutableCardDefault.appearance.tone = 'mutated';
        mutableCardDefault.labels.push('later');

        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={{ type: 'article', content: [] }}
            />
        );

        const expectedEmptyArticle = {
            type: 'article',
            content: [
                {
                    type: 'card',
                    attrs: {
                        config: { appearance: { tone: 'blue' }, labels: ['initial'] },
                    },
                },
            ],
        };
        const controlledRequest = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(controlledRequest.setJson).toEqual(expectedEmptyArticle);

        act(() => ref.current!.clearContent());
        const clearRequest = JSON.parse(
            mockNativeModule.editorV2ReplaceDocument.mock.calls[
                mockNativeModule.editorV2ReplaceDocument.mock.calls.length - 1
            ][1] as string
        ) as Record<string, unknown>;
        expect(clearRequest.setJson).toEqual(expectedEmptyArticle);

        handle.destroy();
    });

    it('drives the retained document API through the v2 handle', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onContentChange = jest.fn();
        const onContentChangeJSON = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onContentChange={onContentChange}
                onContentChangeJSON={onContentChangeJSON}
            />
        );

        expect(getByTestId('native-editor-view')).toBeTruthy();

        expect(ref.current!.getContentJson()).toEqual(V2_INITIAL_DOC);
        expect(ref.current!.getContent()).toBe('<p>hello</p>');
        expect(ref.current!.getIsEmpty()).toBe(false);
        expect(ref.current!.canUndo()).toBe(false);

        act(() => {
            ref.current!.setContent('<p>world</p>');
        });
        expect(mockNativeModule.editorV2ReplaceDocument).toHaveBeenCalledTimes(1);
        const replaceRequest = JSON.parse(
            mockNativeModule.editorV2ReplaceDocument.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(replaceRequest.setHtml).toBe('<p>world</p>');
        expect(replaceRequest.history).toBe('undoableBoundary');
        expect(ref.current!.getContent()).toBe('<p>world</p>');
        expect(onContentChange).toHaveBeenCalledWith('<p>world</p>');
        expect(onContentChangeJSON).toHaveBeenCalledWith(fakeDocForText('world'));

        // Undo state is the engine's, not a TypeScript copy.
        expect(ref.current!.canUndo()).toBe(true);
        act(() => {
            ref.current!.undo();
        });
        expect(ref.current!.getContentJson()).toEqual(V2_INITIAL_DOC);
        expect(ref.current!.canUndo()).toBe(false);

        act(() => {
            ref.current!.setContentJson(V2_DOC_B);
        });
        expect(ref.current!.getContentJson()).toEqual(V2_DOC_B);

        act(() => {
            ref.current!.clearContent();
        });
        expect(ref.current!.getIsEmpty()).toBe(true);
        const clearRequest = JSON.parse(
            mockNativeModule.editorV2ReplaceDocument.mock.calls[
                mockNativeModule.editorV2ReplaceDocument.mock.calls.length - 1
            ][1] as string
        ) as Record<string, unknown>;
        expect(clearRequest.history).toBe('resetAndClear');
        expect(ref.current!.canUndo()).toBe(false);
        expect(ref.current!.getContentJson()).toEqual({
            type: 'doc',
            content: [{ type: 'paragraph' }],
        });

        handle.destroy();
    });

    it('maps controlled valueJSONUpdateMode="replace" to an undoable engine boundary', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='replace'
            />
        );
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_B}
                valueJSONUpdateMode='replace'
            />
        );

        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        const request = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(request.history).toBe('undoableBoundary');
        expect(request.setJson).toEqual(V2_DOC_B);
        expect(request.baseDocumentRevision).toBe('1');
        // Verified through engine undo state, not document content.
        expect(ref.current!.canUndo()).toBe(true);
        // Release the controlled prop first — a controlled value always
        // re-drives the document, so undo is observable uncontrolled.
        rerender(
            <NativeRichTextEditor ref={ref} documentHandle={handle} valueJSONUpdateMode='replace' />
        );
        act(() => {
            ref.current!.undo();
        });
        expect(ref.current!.getContentJson()).toEqual(V2_INITIAL_DOC);
        handle.destroy();
    });

    it('maps controlled valueJSONUpdateMode="reset" to a non-undoable history clear', async () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='replace'
            />
        );
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_B}
                valueJSONUpdateMode='replace'
            />
        );
        expect(ref.current!.canUndo()).toBe(true);
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_C}
                valueJSONUpdateMode='reset'
            />
        );
        await act(async () => Promise.resolve());

        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        const request = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        expect(request.history).toBe('resetAndClear');
        expect(request.setJson).toEqual(V2_DOC_C);
        // The reset cleared the engine undo stack that the replace filled.
        expect(ref.current!.canUndo()).toBe(false);
        expect(ref.current!.getContentJson()).toEqual(V2_DOC_C);
        handle.destroy();
    });

    it('refreshes from the engine after REVISION_MISMATCH and re-applies against the fresh revision', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='replace'
            />
        );
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        // The engine advances behind the component's back (rev 1 -> 2).
        handle.bridge.replaceDocument({ setJson: V2_DOC_B, history: 'undoableBoundary' });

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_C}
                valueJSONUpdateMode='replace'
            />
        );

        // First attempt used the stale cached base revision and was
        // rejected; the component refreshed and re-applied against the
        // engine's actual revision.
        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(2);
        const staleRequest = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[0][1] as string
        ) as Record<string, unknown>;
        const freshRequest = JSON.parse(
            mockNativeModule.editorV2ApplyLocalApi.mock.calls[1][1] as string
        ) as Record<string, unknown>;
        expect(String(staleRequest.baseDocumentRevision)).toBe('1');
        expect(String(freshRequest.baseDocumentRevision)).toBe('2');
        expect(freshRequest.setJson).toEqual(V2_DOC_C);
        expect(ref.current!.getContentJson()).toEqual(V2_DOC_C);
        handle.destroy();
    });
});
