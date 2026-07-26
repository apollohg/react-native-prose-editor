const mockNativeModule = {
    renderDocumentJson: jest.fn(),
    measureContentHeight: jest.fn(),
};
const mockRequireNativeModule = jest.fn(() => mockNativeModule);

jest.mock('expo-modules-core', () => ({
    requireNativeModule: mockRequireNativeModule,
}));

jest.mock('../specs/NativePreparedProseViewer', () => {
    const React = require('react');
    const { View } = require('react-native');

    return React.forwardRef((props: Record<string, unknown>, _ref: React.Ref<unknown>) => (
        <View testID='prepared-prose-viewer' {...props} />
    ));
});

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';

import { NativeProseViewer } from '../NativeProseViewer';

describe('NativeProseViewer', () => {
    beforeEach(() => {
        jest.clearAllMocks();
    });

    afterEach(() => {
        expect(mockRequireNativeModule).not.toHaveBeenCalled();
    });

    it('passes JSON directly to the Fabric component with serialized configuration', () => {
        const document = {
            type: 'doc',
            content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Hello' }] }],
        };

        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={document}
                theme={{ text: { color: '#112233' } }}
                resourceLimits={{ maxSchemaNodes: 500 }}
                imageLoadingPolicy={{ maxPendingRequests: 8 }}
                renderImages={false}
            />
        );

        const nativeProps = getByTestId('prepared-prose-viewer').props;
        expect(nativeProps).toMatchObject({
            sourceKind: 'json',
            source: JSON.stringify(document),
            configJson: expect.any(String),
            themeJson: JSON.stringify({ text: { color: '#112233' } }),
            imagePolicyJson: expect.any(String),
            imagesEnabled: false,
            collapsesWhenEmpty: true,
            enableLinkTaps: true,
            fontEnvironmentRevision: 0,
        });
        expect(JSON.parse(nativeProps.configJson)).toMatchObject({
            initialization: { type: 'localEmpty' },
            limits: { resource: { maxSchemaNodes: 500 } },
        });
        expect(mockNativeModule.renderDocumentJson).not.toHaveBeenCalled();
        expect(mockNativeModule.measureContentHeight).not.toHaveBeenCalled();
    });

    it('passes HTML directly to the Fabric component', () => {
        const { getByTestId } = render(
            <NativeProseViewer contentHTML='<p>Hello from HTML</p>' />
        );

        expect(getByTestId('prepared-prose-viewer').props).toMatchObject({
            sourceKind: 'html',
            source: '<p>Hello from HTML</p>',
            imagesEnabled: true,
        });
    });

    it('reuses serialization for an immutable JSON document', () => {
        const document = Object.freeze({
            type: 'doc',
            content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Cached' }] }],
        });
        const stringifySpy = jest.spyOn(JSON, 'stringify');
        const { rerender } = render(<NativeProseViewer contentJSON={document} />);

        stringifySpy.mockClear();
        rerender(<NativeProseViewer contentJSON={document} />);

        expect(stringifySpy).not.toHaveBeenCalledWith(document);
        stringifySpy.mockRestore();
    });

    it('serializes declarative mention configuration and theme', () => {
        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{ type: 'doc', content: [] }}
                addons={{
                    mentions: {
                        trigger: '@',
                        prefix: '@',
                        theme: { textColor: '#112233', backgroundColor: '#ddeeff' },
                    },
                }}
            />
        );

        const nativeProps = getByTestId('prepared-prose-viewer').props;
        expect(JSON.parse(nativeProps.configJson)).toMatchObject({
            mentions: { trigger: '@', prefix: '@' },
        });
        expect(JSON.parse(nativeProps.themeJson)).toMatchObject({
            mentions: { textColor: '#112233', backgroundColor: '#ddeeff' },
        });
    });

    it('routes native link, mention, and error events through public callbacks', () => {
        const onPressLink = jest.fn();
        const onMentionPress = jest.fn();
        const onError = jest.fn();
        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{ type: 'doc', content: [] }}
                onPressLink={onPressLink}
                addons={{ mentions: { onPress: onMentionPress } }}
                onError={onError}
            />
        );

        const nativeView = getByTestId('prepared-prose-viewer');
        fireEvent(nativeView, 'onPressLink', {
            nativeEvent: { href: 'https://example.com', text: 'Example' },
        });
        fireEvent(nativeView, 'onPressMention', {
            nativeEvent: { docPos: 7, label: '@alice' },
        });
        fireEvent(nativeView, 'onError', {
            nativeEvent: {
                domain: 'viewer',
                code: 'DOCUMENT_INVALID',
                message: 'Invalid document',
                fatal: true,
            },
        });

        expect(onPressLink).toHaveBeenCalledWith({ href: 'https://example.com', text: 'Example' });
        expect(onMentionPress).toHaveBeenCalledWith({ docPos: 7, label: '@alice' });
        expect(onError).toHaveBeenCalledWith({
            domain: 'viewer',
            code: 'DOCUMENT_INVALID',
            message: 'Invalid document',
            fatal: true,
        });
    });

    it('keeps normal ViewProps but has no legacy measurement or bridge props', () => {
        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{ type: 'doc', content: [] }}
                style={{ marginTop: 12 }}
                accessible
                testID='public-viewer'
            />
        );

        const nativeProps = getByTestId('public-viewer').props;
        expect(nativeProps).toMatchObject({ style: { marginTop: 12 }, accessible: true });
        expect(nativeProps.containerWidth).toBeUndefined();
        expect(nativeProps.contentId).toBeUndefined();
        expect(nativeProps.renderJson).toBeUndefined();
        expect(nativeProps.onContentHeightChange).toBeUndefined();
        expect(mockNativeModule.renderDocumentJson).not.toHaveBeenCalled();
        expect(mockNativeModule.measureContentHeight).not.toHaveBeenCalled();
    });
});
