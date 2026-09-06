import './helpers/NativeRichTextEditorFixture';
import { mockNativeModule } from './helpers/NativeRichTextEditorFixture';

import { StyleSheet } from 'react-native';
import { render, act, fireEvent } from '@testing-library/react-native';
import { NativeRichTextEditor } from '../NativeRichTextEditor';
import { createNativeEditorDocumentHandle } from '../NativeEditorBridge';

import { withMentionsSchema } from '../addons';

import { tiptapCompatibleSchema } from '../schemas';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('resolves mention styling with active mark attrs and inserts the resolved mention', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapCompatibleSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [
                        {
                            type: 'paragraph',
                            content: [
                                {
                                    type: 'text',
                                    text: '@al',
                                    marks: [
                                        {
                                            type: 'link',
                                            attrs: { href: 'https://example.test/alice' },
                                        },
                                    ],
                                },
                            ],
                        },
                    ],
                },
            },
        });
        const resolveSelectionAttrs = jest.fn(() => ({ kind: 'user' }));
        const resolveTheme = jest.fn(() => ({ node: { textColor: '#445566' } }));
        const onSelect = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                addons={{
                    mentions: {
                        suggestions: [
                            {
                                key: 'alice',
                                title: 'Alice',
                                attrs: { id: 'user-alice' },
                            },
                        ],
                        resolveSelectionAttrs,
                        resolveTheme,
                        onSelect,
                    },
                }}
            />
        );
        const documentVersion = handle.bridge.getState().documentRevision;

        act(() => {
            getByTestId('native-editor-view').props.onAddonEvent({
                nativeEvent: {
                    editorId: handle.editorId,
                    eventJson: JSON.stringify({
                        type: 'mentionsSelectRequest',
                        trigger: '@',
                        suggestionKey: 'alice',
                        attrs: {
                            id: 'user-alice',
                            label: 'Alice',
                            mentionSuggestionChar: '@',
                        },
                        range: { anchor: 0, head: 3 },
                        documentVersion,
                    }),
                },
            });
        });

        expect(resolveSelectionAttrs).toHaveBeenCalledWith(
            expect.objectContaining({
                attrs: {
                    id: 'user-alice',
                    label: 'Alice',
                    mentionSuggestionChar: '@',
                },
                markAttrs: {
                    link: { href: 'https://example.test/alice' },
                },
            })
        );
        expect(resolveTheme).toHaveBeenCalledWith(
            expect.objectContaining({
                attrs: {
                    id: 'user-alice',
                    label: 'Alice',
                    mentionSuggestionChar: '@',
                    kind: 'user',
                },
                markAttrs: {
                    link: { href: 'https://example.test/alice' },
                },
            })
        );
        const applyCommandCalls = mockNativeModule.editorV2ApplyCommand.mock.calls;
        expect(
            (
                JSON.parse(applyCommandCalls[applyCommandCalls.length - 1][1] as string) as Record<
                    string,
                    unknown
                >
            ).command
        ).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'doc',
                content: [
                    {
                        type: 'mention',
                        attrs: {
                            id: 'user-alice',
                            label: 'Alice',
                            mentionSuggestionChar: '@',
                            kind: 'user',
                            mentionTheme: { node: { textColor: '#445566' } },
                        },
                    },
                    { type: 'text', text: ' ' },
                ],
            },
        });
        expect(onSelect).toHaveBeenCalledWith(
            expect.objectContaining({
                attrs: expect.objectContaining({
                    id: 'user-alice',
                    kind: 'user',
                    mentionTheme: { node: { textColor: '#445566' } },
                }),
            })
        );
        handle.destroy();
    });

    it('feeds the inline toolbar mention suggestions while a query is active', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapCompatibleSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: '@al' }] }],
                },
            },
        });
        const onSelect = jest.fn();
        const { getByTestId, queryByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                toolbarPlacement='inline'
                addons={{
                    mentions: {
                        suggestions: [
                            { key: 'alice', title: 'Alice', attrs: { id: 'user-alice' } },
                        ],
                        onSelect,
                    },
                }}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });

        expect(queryByTestId('editor-toolbar-mention-suggestions')).toBeNull();

        const emitQueryChange = (isActive: boolean) => {
            act(() => {
                view.props.onAddonEvent({
                    nativeEvent: {
                        editorId: handle.editorId,
                        eventJson: JSON.stringify({
                            type: 'mentionsQueryChange',
                            query: 'al',
                            trigger: '@',
                            range: { anchor: 0, head: 3 },
                            isActive,
                            documentVersion: handle.bridge.getState().documentRevision,
                        }),
                    },
                });
            });
        };

        emitQueryChange(true);

        expect(queryByTestId('editor-toolbar-mention-suggestions')).not.toBeNull();

        act(() => {
            fireEvent.press(getByTestId('editor-toolbar-mention-suggestion-alice'));
        });

        // Native adapter parity: a range selection uses Before affinity, in
        // scalar currency. Omitting affinity defaults to After, which Yrs
        // cannot represent for a range.
        const setSelectionCalls = mockNativeModule.editorV2SetSelection.mock.calls;
        expect(
            (
                JSON.parse(setSelectionCalls[setSelectionCalls.length - 1][1] as string) as Record<
                    string,
                    unknown
                >
            ).selection
        ).toEqual({
            type: 'text',
            anchor: { offset: 0, kind: 'scalar', affinity: 'before' },
            head: { offset: 3, kind: 'scalar', affinity: 'before' },
        });

        const applyCommandCalls = mockNativeModule.editorV2ApplyCommand.mock.calls;
        expect(
            (
                JSON.parse(applyCommandCalls[applyCommandCalls.length - 1][1] as string) as Record<
                    string,
                    unknown
                >
            ).command
        ).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'doc',
                content: [
                    {
                        type: 'mention',
                        attrs: {
                            id: 'user-alice',
                            label: 'Alice',
                            mentionSuggestionChar: '@',
                        },
                    },
                    { type: 'text', text: ' ' },
                ],
            },
        });
        expect(onSelect).toHaveBeenCalledWith(
            expect.objectContaining({
                trigger: '@',
                suggestion: expect.objectContaining({ key: 'alice' }),
            })
        );
        expect(queryByTestId('editor-toolbar-mention-suggestions')).toBeNull();

        emitQueryChange(true);
        expect(queryByTestId('editor-toolbar-mention-suggestions')).not.toBeNull();
        emitQueryChange(false);
        expect(queryByTestId('editor-toolbar-mention-suggestions')).toBeNull();

        handle.destroy();
    });

    it('never writes an unrenderable mention theme into the document', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapCompatibleSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: '@al' }] }],
                },
            },
        });
        const consoleError = jest.spyOn(console, 'error').mockImplementation(() => {});
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                toolbarPlacement='inline'
                addons={{
                    mentions: {
                        suggestions: [
                            { key: 'alice', title: 'Alice', attrs: { id: 'user-alice' } },
                        ],
                        // The pre-1.0 flat shape, no longer part of EditorMentionTheme.
                        resolveTheme: () => ({ textColor: '#CC0000' }) as never,
                    },
                }}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        act(() => {
            view.props.onAddonEvent({
                nativeEvent: {
                    editorId: handle.editorId,
                    eventJson: JSON.stringify({
                        type: 'mentionsQueryChange',
                        query: 'al',
                        trigger: '@',
                        range: { anchor: 0, head: 3 },
                        isActive: true,
                        documentVersion: handle.bridge.getState().documentRevision,
                    }),
                },
            });
        });
        act(() => {
            fireEvent.press(getByTestId('editor-toolbar-mention-suggestion-alice'));
        });

        // The document must stay renderable: a rejected theme is dropped, not
        // persisted into content that every later renderUpdate revalidates.
        expect(() => handle.bridge.renderUpdate()).not.toThrow();
        const inserted = mockNativeModule.editorV2ApplyCommand.mock.calls
            .map((call) => JSON.parse(call[1] as string) as Record<string, unknown>)
            .filter(
                (request) =>
                    (request.command as Record<string, unknown>)?.type === 'insertContentJson'
            );
        expect(inserted.length).toBeGreaterThan(0);
        const attrs = (
            (
                (inserted[inserted.length - 1].command as Record<string, unknown>).json as Record<
                    string,
                    Record<string, unknown>[]
                >
            ).content[0] as Record<string, Record<string, unknown>>
        ).attrs;
        expect(attrs.mentionTheme).toBeUndefined();
        expect(consoleError).toHaveBeenCalledWith(
            expect.stringContaining('mentions.resolveTheme'),
            expect.anything()
        );

        consoleError.mockRestore();
        handle.destroy();
    });

    it('falls back to Before affinity when a collapsed mention caret is unrepresentable', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapCompatibleSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: '@' }] }],
                },
            },
        });
        const onSelect = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                toolbarPlacement='inline'
                addons={{
                    mentions: {
                        suggestions: [
                            { key: 'alice', title: 'Alice', attrs: { id: 'user-alice' } },
                        ],
                        onSelect,
                    },
                }}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        act(() => {
            view.props.onAddonEvent({
                nativeEvent: {
                    editorId: handle.editorId,
                    eventJson: JSON.stringify({
                        type: 'mentionsQueryChange',
                        query: '',
                        trigger: '@',
                        range: { anchor: 1, head: 1 },
                        isActive: true,
                        documentVersion: handle.bridge.getState().documentRevision,
                    }),
                },
            });
        });

        // The engine rejects After stickiness at this boundary position.
        mockNativeModule.editorV2SetSelection.mockImplementationOnce(() => ({
            value: null,
            error: {
                domain: 'operation',
                code: 'POSITION_INVALID',
                message: 'selection cannot be represented with the requested Yrs affinity',
                requestId: null,
                operationIndex: null,
                limit: null,
                actual: null,
                details: null,
            },
        }));

        act(() => {
            fireEvent.press(getByTestId('editor-toolbar-mention-suggestion-alice'));
        });

        const calls = mockNativeModule.editorV2SetSelection.mock.calls;
        const affinities = calls
            .slice(-2)
            .map(
                (call) =>
                    (
                        (JSON.parse(call[1] as string) as Record<string, unknown>)
                            .selection as Record<string, Record<string, unknown>>
                    ).anchor.affinity
            );
        expect(affinities).toEqual(['after', 'before']);
        expect(onSelect).toHaveBeenCalledWith(
            expect.objectContaining({ suggestion: expect.objectContaining({ key: 'alice' }) })
        );

        handle.destroy();
    });

    it('applies per-suggestion resolveTheme styling to inline toolbar suggestions', () => {
        const handle = createNativeEditorDocumentHandle({
            schema: withMentionsSchema(tiptapCompatibleSchema),
            initialization: {
                type: 'localJson',
                json: {
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: '@a' }] }],
                },
            },
        });
        const { getByTestId, getByText } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                toolbarPlacement='inline'
                addons={{
                    mentions: {
                        suggestions: [
                            { key: 'channel', title: 'General' },
                            { key: 'alice', title: 'Alice' },
                        ],
                        resolveSelectionAttrs: ({ suggestion, attrs }) => ({
                            ...attrs,
                            type: suggestion.key === 'channel' ? 'channel' : 'user',
                        }),
                        resolveTheme: ({ attrs }) =>
                            attrs.type === 'channel'
                                ? {
                                      node: { textColor: '#00FF00', backgroundColor: '#00FF00' },
                                      suggestions: {
                                          option: {
                                              textColor: '#CC0000',
                                              backgroundColor: '#FFEEEE',
                                          },
                                      },
                                  }
                                : {
                                      suggestions: {
                                          option: {
                                              textColor: '#0000CC',
                                              backgroundColor: '#EEEEFF',
                                          },
                                      },
                                  },
                    },
                }}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        act(() => {
            view.props.onAddonEvent({
                nativeEvent: {
                    editorId: handle.editorId,
                    eventJson: JSON.stringify({
                        type: 'mentionsQueryChange',
                        query: 'a',
                        trigger: '@',
                        range: { anchor: 0, head: 2 },
                        isActive: true,
                        documentVersion: handle.bridge.getState().documentRevision,
                    }),
                },
            });
        });

        const flattenStyle = (element: { props: { style?: unknown } }) =>
            StyleSheet.flatten(element.props.style) as Record<string, unknown>;

        expect(
            flattenStyle(getByTestId('editor-toolbar-mention-suggestion-channel')).backgroundColor
        ).toBe('#FFEEEE');
        expect(
            flattenStyle(getByTestId('editor-toolbar-mention-suggestion-alice')).backgroundColor
        ).toBe('#EEEEFF');
        expect(flattenStyle(getByText('@General')).color).toBe('#CC0000');
        expect(flattenStyle(getByText('@Alice')).color).toBe('#0000CC');

        handle.destroy();
    });
});
