const mockRenderDocumentJson = jest.fn();
const mockMeasureContentHeight = jest.fn();

const mockNativeModule = {
    renderDocumentJson: mockRenderDocumentJson,
    renderDocumentHtml: jest.fn(),
    measureContentHeight: mockMeasureContentHeight,
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
import { PixelRatio, Platform } from 'react-native';

import { NativeProseViewer } from '../NativeProseViewer';
import { clearHeightCache } from '../heightCache';
import * as schemas from '../schemas';

describe('NativeProseViewer', () => {
    let consoleErrorSpy: jest.SpyInstance;

    beforeEach(() => {
        consoleErrorSpy = jest.spyOn(console, 'error').mockImplementation(() => {});
    });

    beforeEach(() => {
        clearHeightCache();
        mockMeasureContentHeight.mockReset();
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
        expect(mockRenderDocumentJson.mock.calls[0]?.[1]).toBe(JSON.stringify(contentJSON));
        expect(nativeView.props.renderJson).toContain('"nodeType":"mention"');
    });

    it('normalizes empty viewer JSON through the custom document descriptor', () => {
        const articleSchema = {
            nodes: [
                { name: 'article', content: 'title+', role: 'doc' },
                { name: 'title', content: 'inline*', group: 'block', role: 'textBlock' },
                { name: 'text', content: '', group: 'inline', role: 'text' },
            ],
            marks: [],
        };

        render(
            <NativeProseViewer
                schema={articleSchema}
                contentJSON={{ type: 'article', content: [] }}
            />
        );

        expect(mockRenderDocumentJson).toHaveBeenCalledWith(
            expect.any(String),
            JSON.stringify({ type: 'article', content: [{ type: 'title' }] })
        );
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

    it('uses the Tiptap normalization fallback for an invalid schema', () => {
        render(
            <NativeProseViewer
                contentJSON={{ type: 'doc', content: [] }}
                schema={{ nodes: [], marks: [] }}
            />
        );

        expect(mockRenderDocumentJson.mock.calls[0]?.[1]).toBe(
            JSON.stringify({ type: 'doc', content: [{ type: 'paragraph' }] })
        );
    });

    it('renders native view from HTML input', () => {
        const contentHTML = '<p>Hello from HTML</p>';

        const { getByTestId } = render(<NativeProseViewer contentHTML={contentHTML} />);

        const nativeView = getByTestId('native-prose-viewer');
        expect(mockNativeModule.renderDocumentHtml).toHaveBeenCalledTimes(1);
        expect(mockNativeModule.renderDocumentHtml.mock.calls[0]?.[1]).toBe(contentHTML);
        expect(mockRenderDocumentJson).not.toHaveBeenCalled();
        expect(nativeView.props.renderJson).toContain('Hello from HTML');
    });

    it('normalizes and passes a configurable image loading policy to native', () => {
        const { getByTestId } = render(
            <NativeProseViewer
                contentHTML='<p>Hello</p>'
                imageLoadingPolicy={{
                    readTimeoutMs: 3000,
                    maxPendingRequests: 8,
                }}
            />
        );

        expect(JSON.parse(getByTestId('native-prose-viewer').props.imageLoadingPolicyJson)).toEqual(
            {
                maxSourceBytes: 10485760,
                connectTimeoutMs: 10000,
                readTimeoutMs: 3000,
                requestTimeoutMs: 60000,
                maxConcurrentRequests: 2,
                maxPendingRequests: 8,
                maxDecodeDimensionPx: 2048,
            }
        );
    });

    it('passes resolved resource limits into the native render config', () => {
        render(
            <NativeProseViewer
                contentJSON={{ type: 'doc' }}
                resourceLimits={{ maxSchemaNodes: 500 }}
            />
        );

        const config = JSON.parse(mockRenderDocumentJson.mock.calls[0]?.[0] as string);
        expect(config.limits.resource).toMatchObject({
            maxInputBytes: 20 * 1024 * 1024,
            maxSchemaNodes: 500,
            maxEncodedStateBytes: 50 * 1024 * 1024,
        });
        expect(config.resourceLimits).toBeUndefined();
    });

    it('does not rescan or rerender for semantically equal inline resource limits', () => {
        const descriptorSpy = jest.spyOn(schemas, 'resolveDocumentDescriptor');
        const contentJSON = { type: 'doc', content: [{ type: 'paragraph' }] };
        const { rerender } = render(
            <NativeProseViewer
                contentJSON={contentJSON}
                resourceLimits={{ maxSchemaNodes: 500 }}
            />
        );

        rerender(
            <NativeProseViewer
                contentJSON={contentJSON}
                resourceLimits={{ maxSchemaNodes: 500 }}
            />
        );

        expect(mockRenderDocumentJson).toHaveBeenCalledTimes(1);
        expect(descriptorSpy).toHaveBeenCalledTimes(1);
        descriptorSpy.mockRestore();
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

    it('resolves mention attrs by doc position before firing the addon press handler', () => {
        const onPress = jest.fn();
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
            <NativeProseViewer contentJSON={contentJSON} addons={{ mentions: { onPress } }} />
        );

        fireEvent(getByTestId('native-prose-viewer'), 'onPressMention', {
            nativeEvent: { docPos: 3, label: '@alice' },
        });

        expect(onPress).toHaveBeenCalledWith({
            docPos: 3,
            label: '@alice',
            attrs: { id: 'user-1', label: '@alice', kind: 'user' },
        });
    });

    it('uses the custom document root when resolving mention positions', () => {
        const onPress = jest.fn();
        const schema = {
            nodes: [
                { name: 'article', content: 'title+', role: 'doc' },
                { name: 'title', content: 'inline*', group: 'block', role: 'textBlock' },
                { name: 'text', content: '', group: 'inline', role: 'text' },
            ],
            marks: [],
        };
        const { getByTestId } = render(
            <NativeProseViewer
                schema={schema}
                contentJSON={{
                    type: 'article',
                    content: [
                        {
                            type: 'title',
                            content: [{ type: 'mention', attrs: { id: '1', label: '@a' } }],
                        },
                    ],
                }}
                addons={{ mentions: { onPress } }}
            />
        );

        fireEvent(getByTestId('native-prose-viewer'), 'onPressMention', {
            nativeEvent: { docPos: 1, label: '@a' },
        });
        expect(onPress).toHaveBeenCalledWith({
            docPos: 1,
            label: '@a',
            attrs: { id: '1', label: '@a' },
        });
    });

    it('applies mention prefixes and per-mention theme overrides before rendering', () => {
        const onPress = jest.fn();
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
                addons={{
                    mentions: {
                        prefix: ({ attrs }) => (attrs.kind === 'user' ? '@' : undefined),
                        resolveTheme: ({ attrs }) =>
                            attrs.id === 'vip-1'
                                ? {
                                      textColor: '#445566',
                                      backgroundColor: '#ddeeff',
                                  }
                                : undefined,
                        onPress,
                    },
                }}
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
            (element) => element.type === 'opaqueInlineAtom' && element.nodeType === 'mention'
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

        expect(onPress).toHaveBeenCalledWith({
            docPos: 7,
            label: '@alice',
            attrs: { id: 'vip-1', label: 'alice', kind: 'user' },
        });
    });

    it('applies mention addon config before rendering', () => {
        const onPress = jest.fn();
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
                addons={{
                    mentions: {
                        trigger: '@',
                        theme: {
                            textColor: '#112233',
                            backgroundColor: '#ddeeff',
                        },
                        resolveTheme: ({ attrs }) =>
                            attrs.id === 'vip-1' ? { fontWeight: 'bold' } : undefined,
                        onPress,
                    },
                }}
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
            (element) => element.type === 'opaqueInlineAtom' && element.nodeType === 'mention'
        );

        expect(renderedMention).toMatchObject({
            label: '@alice',
            mentionTheme: {
                textColor: '#112233',
                backgroundColor: '#ddeeff',
                fontWeight: 'bold',
            },
        });

        fireEvent(nativeView, 'onPressMention', {
            nativeEvent: { docPos: 7, label: 'alice' },
        });

        expect(onPress).toHaveBeenCalledWith({
            docPos: 7,
            label: '@alice',
            attrs: { id: 'vip-1', label: 'alice', kind: 'user' },
        });
    });

    it('routes native mention taps through the mentions addon press handler', () => {
        const onPress = jest.fn();
        const contentJSON = {
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [
                        { type: 'text', text: 'Hello ' },
                        {
                            type: 'mention',
                            attrs: { id: 'user-1', label: 'alice', mentionSuggestionChar: '@' },
                        },
                    ],
                },
            ],
        };

        const { getByTestId } = render(
            <NativeProseViewer contentJSON={contentJSON} addons={{ mentions: { onPress } }} />
        );

        const nativeView = getByTestId('native-prose-viewer');
        expect(nativeView.props.onPressMention).toBeTruthy();

        fireEvent(nativeView, 'onPressMention', {
            nativeEvent: { docPos: 7, label: 'alice' },
        });

        expect(onPress).toHaveBeenCalledWith({
            docPos: 7,
            label: '@alice',
            attrs: { id: 'user-1', label: 'alice', mentionSuggestionChar: '@' },
        });
    });

    it('uses mentionSuggestionChar when viewer addons do not define a mention prefix', () => {
        mockRenderDocumentJson.mockReturnValueOnce(
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

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{
                    type: 'doc',
                    content: [
                        {
                            type: 'paragraph',
                            content: [
                                { type: 'text', text: 'Hello ' },
                                {
                                    type: 'mention',
                                    attrs: {
                                        id: 'user-1',
                                        label: 'alice',
                                        mentionSuggestionChar: '@',
                                    },
                                },
                            ],
                        },
                    ],
                }}
            />
        );

        const renderElements = JSON.parse(
            getByTestId('native-prose-viewer').props.renderJson
        ) as Array<{ type: string; nodeType?: string; label?: string }>;
        const renderedMention = renderElements.find(
            (element) => element.type === 'opaqueInlineAtom' && element.nodeType === 'mention'
        );

        expect(renderedMention?.label).toBe('@alice');
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
        expect(getByTestId('native-prose-viewer').props.collapsesWhenEmpty).toBe(true);
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
        expect(getByTestId('native-prose-viewer').props.collapsesWhenEmpty).toBe(false);
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

    it('converts Android pixel heights to density-independent points', () => {
        const originalPlatform = Platform.OS;
        Object.defineProperty(Platform, 'OS', {
            configurable: true,
            value: 'android',
        });
        const pixelRatioSpy = jest.spyOn(PixelRatio, 'get').mockReturnValue(2.625);

        try {
            const { getByTestId } = render(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [] }],
                    }}
                />
            );

            fireEvent(getByTestId('native-prose-viewer'), 'onContentHeightChange', {
                nativeEvent: { contentHeight: 252 },
            });

            expect(getByTestId('native-prose-viewer').props.style).toEqual([
                { minHeight: 1 },
                undefined,
                { minHeight: Math.ceil(252 / 2.625) },
            ]);
        } finally {
            pixelRatioSpy.mockRestore();
            Object.defineProperty(Platform, 'OS', {
                configurable: true,
                value: originalPlatform,
            });
        }
    });

    it('accepts later positive shrink measurements from native layout stabilization', () => {
        const baseContent = {
            type: 'doc',
            content: [{ type: 'paragraph', content: [] }],
        };
        const { getByTestId, rerender } = render(
            <NativeProseViewer contentJSON={baseContent} contentRevision='first' />
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
            { minHeight: 20 },
        ]);

        rerender(<NativeProseViewer contentJSON={baseContent} contentRevision='second' />);

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
            <NativeProseViewer contentJSON={baseContent} contentJSONRevision='first' />
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
            { minHeight: 20 },
        ]);

        rerender(<NativeProseViewer contentJSON={baseContent} contentJSONRevision='second' />);

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

        expect(consoleErrorSpy).toHaveBeenCalledWith('NativeProseViewer: invalid json');
        expect(getByTestId('native-prose-viewer').props.renderJson).toBe('[]');
    });

    describe('height pre-measurement', () => {
        beforeEach(() => {
            mockMeasureContentHeight.mockReturnValue(84);
        });

        it('uses pre-measured height as initial minHeight when contentId and containerWidth provided', () => {
            const { getByTestId } = render(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [] }],
                    }}
                    contentId='msg-1'
                    containerWidth={375}
                />
            );

            expect(mockMeasureContentHeight).toHaveBeenCalledWith(
                expect.any(String),
                undefined,
                375
            );

            const styles = getByTestId('native-prose-viewer').props.style;
            expect(styles).toEqual(
                expect.arrayContaining([expect.objectContaining({ minHeight: 84 })])
            );
        });

        it('does not call measureContentHeight when contentId is not provided', () => {
            render(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [] }],
                    }}
                    containerWidth={375}
                />
            );

            expect(mockMeasureContentHeight).not.toHaveBeenCalled();
        });

        it('does not call measureContentHeight when containerWidth is not provided', () => {
            render(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [] }],
                    }}
                    contentId='msg-1'
                />
            );

            expect(mockMeasureContentHeight).not.toHaveBeenCalled();
        });

        it('uses cached height on subsequent renders without calling measureContentHeight again', () => {
            const { unmount } = render(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [] }],
                    }}
                    contentId='msg-1'
                    containerWidth={375}
                />
            );

            expect(mockMeasureContentHeight).toHaveBeenCalledTimes(1);
            unmount();
            mockMeasureContentHeight.mockClear();

            const { getByTestId } = render(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [] }],
                    }}
                    contentId='msg-1'
                    containerWidth={375}
                />
            );

            expect(mockMeasureContentHeight).not.toHaveBeenCalled();
            const styles = getByTestId('native-prose-viewer').props.style;
            expect(styles).toEqual(
                expect.arrayContaining([expect.objectContaining({ minHeight: 84 })])
            );
        });

        it('native onContentHeightChange overrides pre-measured height', () => {
            const { getByTestId } = render(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [] }],
                    }}
                    contentId='msg-1'
                    containerWidth={375}
                />
            );

            fireEvent(getByTestId('native-prose-viewer'), 'onContentHeightChange', {
                nativeEvent: { contentHeight: 92 },
            });

            const styles = getByTestId('native-prose-viewer').props.style;
            expect(styles).toEqual([{ minHeight: 1 }, undefined, { minHeight: 92 }]);
        });

        it('falls back to minHeight 1 when no contentId or containerWidth', () => {
            const { getByTestId } = render(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [] }],
                    }}
                />
            );

            expect(getByTestId('native-prose-viewer').props.style).toEqual([
                { minHeight: 1 },
                undefined,
                null,
            ]);
        });
    });

    describe('contentHeight reset on contentId change', () => {
        it('resets contentHeight when contentId changes', () => {
            mockMeasureContentHeight.mockReturnValue(84);
            const { getByTestId, rerender } = render(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [] }],
                    }}
                    contentId='msg-1'
                    containerWidth={375}
                />
            );

            fireEvent(getByTestId('native-prose-viewer'), 'onContentHeightChange', {
                nativeEvent: { contentHeight: 100 },
            });

            expect(getByTestId('native-prose-viewer').props.style).toEqual([
                { minHeight: 1 },
                undefined,
                { minHeight: 100 },
            ]);

            mockMeasureContentHeight.mockReturnValue(60);
            rerender(
                <NativeProseViewer
                    contentJSON={{
                        type: 'doc',
                        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'New' }] }],
                    }}
                    contentId='msg-2'
                    containerWidth={375}
                />
            );

            const styles = getByTestId('native-prose-viewer').props.style;
            const measuredStyle = styles[2];
            expect(measuredStyle?.minHeight).not.toBe(100);
        });
    });
});
