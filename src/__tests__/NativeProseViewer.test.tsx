jest.mock('../specs/PreparedProseViewerNativeComponent', () => {
    const React = require('react');
    const { View } = require('react-native');

    return React.forwardRef((props: Record<string, unknown>, _ref: React.Ref<unknown>) => (
        <View testID='prepared-prose-viewer' {...props} />
    ));
});

import React from 'react';
import { readFileSync } from 'fs';
import { join } from 'path';
import { fireEvent, render } from '@testing-library/react-native';

import { NativeProseViewer } from '../NativeProseViewer';

describe('NativeProseViewer', () => {
    it('benchmark FlatList consumes warmWindows', () => {
        const source = readFileSync(join(__dirname, '..', '..', 'example', 'App.tsx'), 'utf8');
        const benchmarkStart = source.indexOf('function PreparedViewerBenchmarkScreen');
        const benchmarkEnd = source.indexOf('\nconst styles =', benchmarkStart);
        expect(benchmarkStart).toBeGreaterThanOrEqual(0);
        expect(benchmarkEnd).toBeGreaterThan(benchmarkStart);
        const benchmark = source.slice(benchmarkStart, benchmarkEnd);

        expect(source).toContain('warmWindows: WarmWindow[]');
        expect(source).toContain('type ScrollCommandToken');
        expect(source).toContain('SCROLL_COMMAND_NO_MOTION_TIMEOUT_MS');
        expect(source).toContain('SCROLL_COMMAND_MIN_OFFSET_DELTA');
        expect(benchmark).toContain('preparedViewerCorpus.warmWindows');
        expect(benchmark).toContain('windowIndex');
        expect(benchmark).toContain('phase');
        expect(benchmark).toContain('direction');
        expect(benchmark).toContain('scrollToEnd({ animated: true })');
        expect(benchmark).toContain('scrollToIndex({ index: 0, animated: true })');
        for (const lifecycleContract of [
            'dispatched',
            'momentumBegan',
            'consumed',
            'expectedDirection',
            'expectedTerminalEntryId',
            'dispatchOffsetY',
            'latestNativeContentOffsetYRef',
            'SCROLL_COMMAND_NO_MOTION_TIMEOUT_MS',
            'SCROLL_COMMAND_MIN_OFFSET_DELTA',
            'dispatchScrollCommand',
            'clearScrollCommandWatchdog',
            'releaseActiveTraversal',
            'cancelActiveTraversal',
            'handleViewableItemsChanged',
            'handleScroll',
            'onScroll={handleScroll}',
            'onViewableItemsChanged={handleViewableItemsChanged}',
            'onMomentumScrollBegin={handleMomentumScrollBegin}',
            'onMomentumScrollEnd={handleMomentumScrollEnd}',
            'nativeEvent.contentOffset.y',
        ]) {
            expect(benchmark).toContain(lifecycleContract);
        }
        expect(benchmark).toContain('const dispatchOffsetY = latestNativeContentOffsetYRef.current;');
        expect(benchmark).toContain('const offsetDelta = event.nativeEvent.contentOffset.y - command.dispatchOffsetY;');
        expect(benchmark).toContain('command.momentumBegan ||');
        expect(benchmark).not.toContain('startOffsetY');
        expect(benchmark).not.toContain('command.dispatchOffsetY =');
        // Memoized, not inline: this screen's whole contract is keeping React
        // reconciliation cost out of the window it measures.
        expect(benchmark).toContain('const keyExtractor = useCallback((item: CorpusEntry) => item.id, []);');
        expect(benchmark).toContain('keyExtractor={keyExtractor}');
        expect(benchmark).toContain('renderImages={imagesEnabled}');
        for (const bridgeMethod of [
            'preparedProseBenchmarkBegin',
            'preparedProseBenchmarkBeginPhase',
            'preparedProseBenchmarkEndPhase',
            'preparedProseBenchmarkReset',
            'preparedProseBenchmarkExport',
        ]) {
            expect(source).toContain(bridgeMethod);
        }
        for (const forbidden of [
            'onContentSizeChange',
            'contentHeightRef',
            'scrollToOffset',
            'measureInWindow',
            'onContentHeightChange',
            'heightCache',
            'containerWidth',
            'getItemLayout',
        ]) {
            expect(benchmark).not.toContain(forbidden);
        }
        expect(benchmark).not.toMatch(/\bmeasure\s*\(/);
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
                addons={{ mentions: { onPress: onMentionPress } }}
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
