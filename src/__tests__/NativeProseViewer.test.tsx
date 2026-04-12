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

    it('updates the native view height when content height changes', () => {
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
            { height: 84 },
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
