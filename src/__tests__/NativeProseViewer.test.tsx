import { createMentionsAddon } from '../EditorAddon';
jest.mock('../specs/PreparedProseViewerNativeComponent', () => {
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
    it('updates callbacks per viewer and clears removed addon capabilities', () => {
        const first = jest.fn();
        const next = jest.fn();
        const shared = createMentionsAddon({ prefix: '@', onPress: first });
        const highlighting = {
            id: 'code-highlighting',
            version: 1,
            capability: 'code-highlighting',
            options: { provider: 'syntect', theme: 'base16-ocean.dark' },
        } as const;
        const contentJSON = { type: 'doc', content: [] };
        const viewers = (addons: import('../EditorAddon').EditorAddons) => (
            <>
                <NativeProseViewer
                    testID='first-viewer'
                    contentJSON={contentJSON}
                    addons={addons}
                />
                <NativeProseViewer
                    testID='other-viewer'
                    contentJSON={contentJSON}
                    addons={[shared]}
                />
            </>
        );
        const { getByTestId, rerender } = render(viewers([shared, highlighting]));
        const press = (testID: string) =>
            fireEvent(getByTestId(testID), 'onPressMention', {
                nativeEvent: { docPos: 1, label: 'Alice', attrsJson: '{}' },
            });
        const oldConfig = getByTestId('first-viewer').props.configJson;
        expect(JSON.parse(oldConfig).codeHighlighting).toEqual(highlighting.options);
        rerender(viewers([createMentionsAddon({ prefix: '@', onPress: next }), highlighting]));
        expect(getByTestId('first-viewer').props.configJson).toBe(oldConfig);
        press('first-viewer');
        press('other-viewer');
        expect(first).toHaveBeenCalledTimes(1);
        expect(next).toHaveBeenCalledTimes(1);
        rerender(viewers([]));
        expect(getByTestId('first-viewer').props.mentionInteractionsEnabled).toBe(false);
        expect(getByTestId('first-viewer').props.onPressMention).toBeUndefined();
        expect(
            JSON.parse(getByTestId('first-viewer').props.configJson).codeHighlighting
        ).toBeUndefined();
        expect(getByTestId('other-viewer').props.mentionInteractionsEnabled).toBe(true);
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
            themeJson: JSON.stringify({ version: 1, styles: { text: { color: '#112233ff' } } }),
            imagePolicyJson: expect.any(String),
            imagesEnabled: false,
            collapsesWhenEmpty: true,
            enableLinkTaps: false,
            mentionInteractionsEnabled: false,
            fontEnvironmentRevision: 0,
        });
        expect(JSON.parse(nativeProps.configJson)).toMatchObject({
            initialization: { type: 'localEmpty' },
            limits: { resource: { maxSchemaNodes: 500 } },
        });
    });

    it('passes HTML directly to the Fabric component', () => {
        const { getByTestId } = render(<NativeProseViewer contentHTML='<p>Hello from HTML</p>' />);

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
                addons={[
                    createMentionsAddon({
                        trigger: '@',
                        prefix: '@',
                        theme: { node: { textColor: '#112233', backgroundColor: '#ddeeff' } },
                    }),
                ]}
            />
        );

        const nativeProps = getByTestId('prepared-prose-viewer').props;
        expect(JSON.parse(nativeProps.configJson)).toMatchObject({
            mentions: { trigger: '@', prefix: '@' },
        });
        expect(JSON.parse(nativeProps.themeJson)).toMatchObject({
            mentions: { node: { style: { color: '#112233ff', backgroundColor: '#ddeeffff' } } },
        });
    });

    it('passes only effective link and mention interaction capabilities', () => {
        const onPressLink = jest.fn();
        const onPressMention = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeProseViewer contentJSON={{ type: 'doc', content: [] }} enableLinkTaps />
        );

        expect(getByTestId('prepared-prose-viewer').props).toMatchObject({
            enableLinkTaps: false,
            mentionInteractionsEnabled: false,
        });

        rerender(
            <NativeProseViewer
                contentJSON={{ type: 'doc', content: [] }}
                enableLinkTaps
                onPressLink={onPressLink}
            />
        );
        expect(getByTestId('prepared-prose-viewer').props).toMatchObject({
            enableLinkTaps: true,
            mentionInteractionsEnabled: false,
        });

        rerender(
            <NativeProseViewer
                contentJSON={{ type: 'doc', content: [] }}
                enableLinkTaps={false}
                onPressLink={onPressLink}
                addons={[createMentionsAddon({ onPress: onPressMention })]}
            />
        );
        expect(getByTestId('prepared-prose-viewer').props).toMatchObject({
            enableLinkTaps: false,
            mentionInteractionsEnabled: true,
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
                addons={[createMentionsAddon({ onPress: onMentionPress })]}
                onError={onError}
            />
        );

        const nativeView = getByTestId('prepared-prose-viewer');
        fireEvent(nativeView, 'onPressLink', {
            nativeEvent: { href: 'https://example.com', text: 'Example' },
        });
        fireEvent(nativeView, 'onPressMention', {
            nativeEvent: {
                docPos: 4_294_967_295,
                label: '@alice',
                attrsJson: '{"id":"user-9","profile":{"kind":"clinician"}}',
            },
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
        expect(onMentionPress).toHaveBeenCalledWith({
            docPos: 4_294_967_295,
            label: '@alice',
            attrs: { id: 'user-9', profile: { kind: 'clinician' } },
        });
        expect(onError).toHaveBeenCalledWith({
            domain: 'viewer',
            code: 'DOCUMENT_INVALID',
            message: 'Invalid document',
            fatal: true,
        });
    });

    it('rejects non-object mention attributes without invoking the mention callback', () => {
        const onMentionPress = jest.fn();
        const onError = jest.fn();
        const { getByTestId } = render(
            <NativeProseViewer
                contentJSON={{ type: 'doc', content: [] }}
                addons={[createMentionsAddon({ onPress: onMentionPress })]}
                onError={onError}
            />
        );

        fireEvent(getByTestId('prepared-prose-viewer'), 'onPressMention', {
            nativeEvent: { docPos: 9, label: '@alice', attrsJson: '[]' },
        });

        expect(onMentionPress).not.toHaveBeenCalled();
        expect(onError).toHaveBeenCalledWith({
            domain: 'viewer',
            code: 'INVALID_MENTION_ATTRIBUTES',
            message: 'The prepared mention attributes are not a JSON object.',
            fatal: false,
        });
    });

    it('passes normal ViewProps through to the Fabric component', () => {
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
    });
});
