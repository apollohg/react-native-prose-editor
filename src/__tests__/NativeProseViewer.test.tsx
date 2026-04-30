const mockRenderDocumentJson = jest.fn();

const mockNativeModule = {
    renderDocumentJson: mockRenderDocumentJson,
    renderDocumentHtml: jest.fn(),
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
        mockNativeModule.renderDocumentHtml.mockReset();
        mockNativeModule.renderDocumentHtml.mockReturnValue(
            JSON.stringify([
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: 'Hello from HTML', marks: [] },
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

    it('renders native view from HTML input', () => {
        const contentHTML = '<p>Hello from HTML</p>';

        const { getByTestId } = render(<NativeProseViewer contentHTML={contentHTML} />);

        const nativeView = getByTestId('native-prose-viewer');
        expect(mockNativeModule.renderDocumentHtml).toHaveBeenCalledTimes(1);
        expect(mockNativeModule.renderDocumentHtml.mock.calls[0]?.[1]).toBe(
            contentHTML
        );
        expect(mockRenderDocumentJson).not.toHaveBeenCalled();
        expect(nativeView.props.renderJson).toContain('Hello from HTML');
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

    it('enables native link taps by default', () => {
        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
            />
        );

        expect(getByTestId('native-prose-viewer').props.enableLinkTaps).toBe(true);
        expect(getByTestId('native-prose-viewer').props.interceptLinkTaps).toBe(false);
    });

    it('routes native link taps through onPressLink when provided', () => {
        const onPressLink = jest.fn();

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
                onPressLink={onPressLink}
            />
        );

        const nativeView = getByTestId('native-prose-viewer');
        expect(nativeView.props.enableLinkTaps).toBe(true);
        expect(nativeView.props.interceptLinkTaps).toBe(true);

        fireEvent(nativeView, 'onPressLink', {
            nativeEvent: { href: 'https://example.com', text: 'Example' },
        });

        expect(onPressLink).toHaveBeenCalledWith({
            href: 'https://example.com',
            text: 'Example',
        });
    });

    it('collapses trailing empty paragraphs by default', () => {
        mockRenderDocumentJson.mockReturnValueOnce(
            JSON.stringify([
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: 'Hello', marks: [] },
                { type: 'blockEnd' },
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: '\u200B', marks: [] },
                { type: 'blockEnd' },
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: '\u200B', marks: [] },
                { type: 'blockEnd' },
            ])
        );

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
            />
        );

        expect(JSON.parse(getByTestId('native-prose-viewer').props.renderJson)).toEqual([
            { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
            { type: 'textRun', text: 'Hello', marks: [] },
            { type: 'blockEnd' },
        ]);
        expect(getByTestId('native-prose-viewer').props.collapsesWhenEmpty).toBe(
            true
        );
    });

    it('preserves trailing empty paragraphs when collapseTrailingEmptyParagraphs is false', () => {
        mockRenderDocumentJson.mockReturnValueOnce(
            JSON.stringify([
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: 'Hello', marks: [] },
                { type: 'blockEnd' },
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: '\u200B', marks: [] },
                { type: 'blockEnd' },
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: '\u200B', marks: [] },
                { type: 'blockEnd' },
            ])
        );

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
                collapseTrailingEmptyParagraphs={false}
            />
        );

        expect(JSON.parse(getByTestId('native-prose-viewer').props.renderJson)).toEqual([
            { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
            { type: 'textRun', text: 'Hello', marks: [] },
            { type: 'blockEnd' },
            { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
            { type: 'textRun', text: '\u200B', marks: [] },
            { type: 'blockEnd' },
            { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
            { type: 'textRun', text: '\u200B', marks: [] },
            { type: 'blockEnd' },
        ]);
        expect(getByTestId('native-prose-viewer').props.collapsesWhenEmpty).toBe(
            false
        );
    });

    it('collapses all-empty paragraph documents to zero height by default', () => {
        mockRenderDocumentJson.mockReturnValueOnce(
            JSON.stringify([
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: '\u200B', marks: [] },
                { type: 'blockEnd' },
            ])
        );

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
            />
        );

        expect(getByTestId('native-prose-viewer').props.style).toEqual([
            { minHeight: 0 },
            undefined,
            { height: 0, minHeight: 0 },
        ]);
    });

    it('keeps all-empty paragraph height measurable when collapseTrailingEmptyParagraphs is false', () => {
        mockRenderDocumentJson.mockReturnValueOnce(
            JSON.stringify([
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: '\u200B', marks: [] },
                { type: 'blockEnd' },
            ])
        );

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
                collapseTrailingEmptyParagraphs={false}
            />
        );

        expect(getByTestId('native-prose-viewer').props.style).toEqual([
            { minHeight: 1 },
            undefined,
            null,
        ]);
    });

    it('accepts zero-height native measurements for collapsed empty documents', () => {
        mockRenderDocumentJson.mockReturnValueOnce(
            JSON.stringify([
                { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
                { type: 'textRun', text: '\u200B', marks: [] },
                { type: 'blockEnd' },
            ])
        );

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [{ type: 'paragraph', content: [] }],
                }}
            />
        );

        fireEvent(getByTestId('native-prose-viewer'), 'onContentHeightChange', {
            nativeEvent: { contentHeight: 0 },
        });

        expect(getByTestId('native-prose-viewer').props.style).toEqual([
            { minHeight: 0 },
            undefined,
            { height: 0, minHeight: 0 },
        ]);
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
                contentRevision='first'
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
                contentRevision='second'
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

    it('accepts contentJSONRevision as a compatibility alias for contentRevision', () => {
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
        fireEvent(getByTestId('native-prose-viewer'), 'onContentHeightChange', {
            nativeEvent: { contentHeight: 20 },
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
            nativeEvent: { contentHeight: 52 },
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
