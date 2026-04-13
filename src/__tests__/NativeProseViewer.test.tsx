const mockRenderDocumentJson = jest.fn();

const mockNativeModule = {
    renderDocumentJson: mockRenderDocumentJson,
};

jest.mock('expo-modules-core', () => {
    const React = require('react');
    const { View } = require('react-native');

    const MockViewerView = React.forwardRef(
        (props: Record<string, unknown>, _ref: React.Ref<unknown>) => (
            <View testID='native-prose-viewer' {...props} />
        )
    );

    return {
        requireNativeModule: () => mockNativeModule,
        requireNativeViewManager: (moduleName: string, viewName?: string) => {
            if (moduleName === 'NativeEditor' && viewName === 'NativeProseViewer') {
                return MockViewerView;
            }
            throw new Error(
                `Unexpected native view manager request: ${moduleName} ${viewName ?? ''}`.trim()
            );
        },
    };
});

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';

import { NativeProseViewer } from '../NativeProseViewer';

describe('NativeProseViewer', () => {
    let consoleErrorSpy: jest.SpyInstance;

    beforeEach(() => {
        consoleErrorSpy = jest.spyOn(console, 'error').mockImplementation(() => {});
    });

    beforeEach(() => {
        mockRenderDocumentJson.mockReset();
        mockRenderDocumentJson.mockReturnValue(
            JSON.stringify([
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: 'Hello ', marks: [] },
                {
                    type: 'opaqueInlineAtom',
                    nodeType: 'mention',
                    label: '@alice',
                    docPos: 7,
                },
                { type: 'blockEnd' },
            ])
        );
    });

    afterEach(() => {
        consoleErrorSpy.mockRestore();
    });

    it('renders native view with render JSON from the native module', () => {
        const contentJSON = {
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [
                        { type: 'text', text: 'Hello ' },
                        { type: 'mention', attrs: { id: 'user-1', label: '@alice' } },
                    ],
                },
            ],
        };

        const { getByTestId } = render(<NativeProseViewer contentJSON={contentJSON} />);

        const nativeView = getByTestId('native-prose-viewer');
        expect(mockRenderDocumentJson).toHaveBeenCalledTimes(1);
        expect(mockRenderDocumentJson.mock.calls[0]?.[1]).toBe(
            JSON.stringify(contentJSON)
        );
        expect(nativeView.props.renderJson).toContain('"nodeType":"mention"');
    });

    it('accepts serialized document JSON strings', () => {
        const contentJSON = JSON.stringify({
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [{ type: 'text', text: 'Hello from string input' }],
                },
            ],
        });

        render(<NativeProseViewer contentJSON={contentJSON} />);

        expect(mockRenderDocumentJson.mock.calls[0]?.[1]).toBe(contentJSON);
    });

    it('includes mention schema support in the native render config', () => {
        render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
            />
        );

        const config = JSON.parse(mockRenderDocumentJson.mock.calls[0]?.[0] as string) as {
            schema: {
                nodes: Array<{ name: string }>;
            };
        };
        expect(config.schema.nodes.some((node) => node.name === 'mention')).toBe(true);
    });

    it('resolves mention attrs by doc position before firing onPressMention', () => {
        const onPressMention = jest.fn();
        const contentJSON = {
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [
                        { type: 'text', text: '😀 ' },
                        {
                            type: 'mention',
                            attrs: { id: 'user-1', label: '@alice', kind: 'user' },
                        },
                    ],
                },
            ],
        };

        const { getByTestId } = render(
            <NativeProseViewer contentJSON={contentJSON} onPressMention={onPressMention} />
        );

        fireEvent(getByTestId('native-prose-viewer'), 'onPressMention', {
            nativeEvent: { docPos: 3, label: '@alice' },
        });

        expect(onPressMention).toHaveBeenCalledWith({
            docPos: 3,
            label: '@alice',
            attrs: { id: 'user-1', label: '@alice', kind: 'user' },
        });
    });

    it('applies mention prefixes and per-mention theme overrides before rendering', () => {
        const onPressMention = jest.fn();
        mockRenderDocumentJson.mockReturnValueOnce(
            JSON.stringify([
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: 'Hello ', marks: [] },
                {
                    type: 'opaqueInlineAtom',
                    nodeType: 'mention',
                    label: 'alice',
                    docPos: 7,
                },
                { type: 'blockEnd' },
            ])
        );
        const contentJSON = {
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [
                        { type: 'text', text: 'Hello ' },
                        {
                            type: 'mention',
                            attrs: { id: 'vip-1', label: 'alice', kind: 'user' },
                        },
                    ],
                },
            ],
        };

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={contentJSON}
                mentionPrefix={({ attrs }) =>
                    attrs.kind === 'user' ? '@' : undefined
                }
                resolveMentionTheme={({ attrs }) =>
                    attrs.id === 'vip-1'
                        ? {
                              textColor: '#445566',
                              backgroundColor: '#ddeeff',
                          }
                        : undefined
                }
                onPressMention={onPressMention}
            />
        );

        const nativeView = getByTestId('native-prose-viewer');
        const renderElements = JSON.parse(nativeView.props.renderJson) as Array<{
            type: string;
            nodeType?: string;
            label?: string;
            mentionTheme?: Record<string, unknown>;
        }>;
        const renderedMention = renderElements.find(
            (element) =>
                element.type === 'opaqueInlineAtom' && element.nodeType === 'mention'
        );

        expect(renderedMention).toMatchObject({
            label: '@alice',
            mentionTheme: {
                textColor: '#445566',
                backgroundColor: '#ddeeff',
            },
        });

        fireEvent(nativeView, 'onPressMention', {
            nativeEvent: { docPos: 7, label: 'alice' },
        });

        expect(onPressMention).toHaveBeenCalledWith({
            docPos: 7,
            label: '@alice',
            attrs: { id: 'vip-1', label: 'alice', kind: 'user' },
        });
    });

    it('applies measured content height as a minimum height', () => {
        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
            />
        );

        fireEvent(getByTestId('native-prose-viewer'), 'onContentHeightChange', {
            nativeEvent: { contentHeight: 84 },
        });

        expect(getByTestId('native-prose-viewer').props.style).toEqual([
            { minHeight: 1 },
            undefined,
            { minHeight: 84 },
        ]);
    });

    it('ignores stale smaller measurements until the rendered content changes', () => {
        const baseContent = {
            type: 'doc',
            content: [{ type: 'paragraph', content: [] }],
        };
        const { getByTestId, rerender } = render(
            <NativeProseViewer
                contentJSON={baseContent}
                contentJSONRevision='first'
            />
        );

        fireEvent(getByTestId('native-prose-viewer'), 'onContentHeightChange', {
            nativeEvent: { contentHeight: 84 },
        });

        expect(getByTestId('native-prose-viewer').props.style).toEqual([
            { minHeight: 1 },
            undefined,
            { minHeight: 84 },
        ]);

        fireEvent(getByTestId('native-prose-viewer'), 'onContentHeightChange', {
            nativeEvent: {
                contentHeight: 20,
            },
        });

        expect(getByTestId('native-prose-viewer').props.style).toEqual([
            { minHeight: 1 },
            undefined,
            { minHeight: 84 },
        ]);

        rerender(
            <NativeProseViewer
                contentJSON={baseContent}
                contentJSONRevision='second'
            />
        );

        fireEvent(getByTestId('native-prose-viewer'), 'onContentHeightChange', {
            nativeEvent: {
                contentHeight: 52,
            },
        });

        expect(getByTestId('native-prose-viewer').props.style).toEqual([
            { minHeight: 1 },
            undefined,
            { minHeight: 52 },
        ]);
    });

    it('logs native render errors and falls back to an empty render', () => {
        mockRenderDocumentJson.mockReturnValue('{"error":"invalid json"}');

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
            />
        );

        expect(consoleErrorSpy).toHaveBeenCalledWith(
            'NativeProseViewer: invalid json'
        );
        expect(getByTestId('native-prose-viewer').props.renderJson).toBe('[]');
    });
});
