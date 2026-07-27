import * as fs from 'fs';
import * as path from 'path';

const REPO_ROOT = path.resolve(__dirname, '..', '..');

function readSource(relativePath: string): string {
    return fs.readFileSync(path.join(REPO_ROOT, relativePath), 'utf8');
}

function methodBody(source: string, signature: string): string {
    const start = source.indexOf(signature);
    expect(start).toBeGreaterThanOrEqual(0);
    const nextMethod = source.indexOf('\n- (void)', start + signature.length);
    return source.slice(start, nextMethod === -1 ? source.length : nextMethod);
}

function visitCorpusNode(
    node: unknown,
    nodeTypes: Set<string>,
    markTypes: Set<string>,
    state: { nestedList: boolean; imageAttrs: Set<string>; orderedStart: boolean; checkedItem: boolean }
): void {
    if (node == null || typeof node !== 'object' || Array.isArray(node)) return;
    const value = node as { type?: unknown; attrs?: unknown; marks?: unknown; content?: unknown };
    if (typeof value.type === 'string') nodeTypes.add(value.type);
    if (value.type === 'orderedList' && value.attrs != null && typeof value.attrs === 'object') {
        state.orderedStart = Object.prototype.hasOwnProperty.call(value.attrs, 'start');
    }
    if (value.type === 'listItem' && value.attrs != null && typeof value.attrs === 'object') {
        state.checkedItem ||= Object.prototype.hasOwnProperty.call(value.attrs, 'checked');
    }
    if (value.type === 'image' && value.attrs != null && typeof value.attrs === 'object') {
        Object.keys(value.attrs as Record<string, unknown>).forEach((name) => state.imageAttrs.add(name));
    }
    if (Array.isArray(value.marks)) {
        value.marks.forEach((mark) => {
            if (mark != null && typeof mark === 'object' && typeof (mark as { type?: unknown }).type === 'string') {
                markTypes.add((mark as { type: string }).type);
            }
        });
    }
    if (Array.isArray(value.content)) {
        const children = value.content as unknown[];
        if (value.type === 'listItem') {
            state.nestedList ||= children.some((child) => {
                const type = child != null && typeof child === 'object' ? (child as { type?: unknown }).type : undefined;
                return type === 'orderedList' || type === 'bulletList' || type === 'taskList';
            });
        }
        children.forEach((child) => visitCorpusNode(child, nodeTypes, markTypes, state));
    }
}

describe('prepared prose native lifecycle contracts', () => {
    it('keeps an iOS Fabric artifact generation intact for link-permission-only updates', () => {
        const source = readSource('ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm');
        const generationComparator = methodBody(source, 'bool HasEquivalentGenerationProps(');
        const updateProps = methodBody(source, '- (void)updateProps:');

        expect(generationComparator).not.toContain('enableLinkTaps');
        expect(updateProps).toContain('const BOOL generationChanged =');
        expect(updateProps).toContain('if (generationChanged) {\n    [self beginNewGeneration];\n  }');
        expect(updateProps).toContain('_drawingView.linkInteractionsEnabled = nextProps->enableLinkTaps;');
        expect(updateProps).toContain(
            'if (generationChanged && _hasReceivedUsableLayoutMetrics) {\n    [self installMeasuredArtifactIfAttached];\n  }'
        );
    });

    it('publishes exactly one iOS accessibility invalidation for a permission change', () => {
        const source = readSource('ios/Viewer/PreparedProseDrawingView.swift');
        const setter = source.slice(
            source.indexOf('var linkInteractionsEnabled = true'),
            source.indexOf('private var accessibilityElementsByIndex')
        );

        expect(setter.match(/invalidateAccessibilityNodes\(\)/g)).toHaveLength(1);
    });

    it('starts Fabric semantic image ownership before a mount artifact can bind ordinal descriptors', () => {
        const ios = readSource('ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm');
        const iosDrawing = readSource('ios/Viewer/PreparedProseDrawingView.swift');
        const android = readSource('android/src/main/java/com/apollohg/editor/viewer/PreparedProseViewerManager.kt');
        const iosProps = methodBody(ios, '- (void)updateProps:');
        const iosState = methodBody(ios, '- (void)updateState:');
        const androidUpdate = android.slice(android.indexOf('private fun update('), android.indexOf('private fun installCachedLayout('));
        const androidMount = android.slice(android.indexOf('fun beginImages('), android.indexOf('fun requestVisibleImages('));
        const androidMeasure = android.slice(android.indexOf('override fun measure('), android.indexOf('private fun update('));

        expect(iosProps.indexOf('_viewerProps = nextProps;')).toBeLessThan(iosProps.indexOf('[self beginSemanticImageGenerationIfPossible];'));
        expect(iosState.indexOf('_viewerState = nextState;')).toBeLessThan(iosState.indexOf('[self beginSemanticImageGenerationIfPossible];'));
        expect(iosDrawing).toContain('@objc(beginSemanticImageGeneration:)');
        expect(androidUpdate.indexOf('state.mutation()')).toBeLessThan(androidUpdate.indexOf('state.beginSemanticImageGeneration(view)'));
        expect(androidUpdate.indexOf('state.beginSemanticImageGeneration(view)')).toBeLessThan(androidUpdate.indexOf('reconcile(view, state)'));
        expect(androidMeasure.indexOf('FabricAttachmentSidecars.begin(it, request.semanticGenerationIdentity)')).toBeLessThan(
            androidMeasure.indexOf('PreparedProseLayoutRegistry.shared.measure(')
        );
        expect(androidMount).not.toContain('attachmentRevisions.beginSemanticGeneration');
    });

    it('drops Android Fabric sidecars through the persisted token after view tags mutate', () => {
        const android = readSource('android/src/main/java/com/apollohg/editor/viewer/PreparedProseViewerManager.kt');
        const drop = android.slice(android.indexOf('override fun onDropViewInstance'), android.indexOf('override fun onSurfaceStopped'));
        const viewState = android.slice(android.indexOf('private class ViewState'), android.indexOf('\n    companion object'));
        const adopt = viewState.slice(viewState.indexOf('fun adopt('), viewState.indexOf('\n        fun release()'));
        const release = viewState.slice(viewState.indexOf('fun release()'), viewState.indexOf('\n    }\n\n    companion object'));
        const releaseSidecar = viewState.slice(viewState.indexOf('private fun releaseSidecarOwnership()'));

        expect(drop).toContain('state.release()');
        expect(drop).not.toContain('UIManagerHelper.getSurfaceId(view)');
        expect(drop).not.toContain('FabricAttachmentSidecars.remove(');
        expect(viewState).toContain('private var sidecarGeneration: FabricGenerationToken? = null');
        expect(adopt).toContain('previousSidecar.surface != next.surface');
        expect(adopt).toContain('releaseFabricSurface(previousSidecar.surface)');
        expect(release).toContain('releaseSidecarOwnership()');
        expect(releaseSidecar).toContain('val sidecar = sidecarGeneration ?: return');
        expect(releaseSidecar).toContain('sidecarGeneration = null');
        expect(releaseSidecar).toContain('releaseFabricSurface(sidecar.surface)');
    });

    it('releases iOS Fabric sidecars on detach, recycle, and dealloc using only recorded ownership', () => {
        const ios = readSource('ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm');
        const detach = methodBody(ios, '- (void)didMoveToSuperview');
        const recycle = methodBody(ios, '- (void)prepareForRecycle');
        const dealloc = methodBody(ios, '- (void)dealloc');
        const releaseSidecar = methodBody(ios, '- (void)releaseFabricSidecarOwnership');
        const install = methodBody(ios, '- (void)installMeasuredArtifactIfAttached');

        expect(detach).toContain('[self releaseAllFabricOwnership];');
        expect(detach).not.toContain('[self releaseFabricOwnership];');
        expect(recycle).toContain('[self releaseAllFabricOwnership];');
        expect(dealloc).toContain('[self releaseAllFabricOwnership];');
        expect(releaseSidecar).toContain('if (!_hasOwnedSidecar) return;');
        expect(releaseSidecar).toContain('releaseFabricSurfaceId:_ownedSidecarSurfaceId');
        expect(releaseSidecar).toContain('componentTag:_ownedSidecarComponentTag');
        expect(releaseSidecar).toContain('_hasOwnedSidecar = NO;');
        expect(releaseSidecar).not.toContain('self.tag');
        expect(install).toContain('[self releaseFabricSidecarOwnership];');
    });

    it('keeps every very-long literal independently complete and preserves the exact traversal contract', () => {
        const corpus = JSON.parse(readSource('scripts/tests/viewer-performance-corpus.json')) as {
            documents: Array<{ id: string; category: string; contentJSON: unknown }>;
            coldTraversal: string[];
            warmTraversal: string[];
        };
        expect(corpus.documents).toHaveLength(1_000);
        expect(new Set(corpus.documents.map(({ id }) => id)).size).toBe(1_000);
        expect(corpus.documents.filter(({ category }) => category === 'short')).toHaveLength(900);
        expect(corpus.documents.filter(({ category }) => category === 'medium-multi-block')).toHaveLength(80);
        expect(corpus.documents.filter(({ category }) => category === 'image-bearing')).toHaveLength(15);
        const longEntries = corpus.documents.filter(({ category }) => category === 'very-long-all-elements-marks');
        expect(longEntries).toHaveLength(5);
        expect(corpus.coldTraversal).toEqual(corpus.documents.map(({ id }) => id));
        expect(corpus.warmTraversal).toEqual([...corpus.coldTraversal].reverse());

        for (const entry of longEntries) {
            const nodeTypes = new Set<string>();
            const markTypes = new Set<string>();
            const state = { nestedList: false, imageAttrs: new Set<string>(), orderedStart: false, checkedItem: false };
            visitCorpusNode(entry.contentJSON, nodeTypes, markTypes, state);
            [
                'paragraph', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'codeBlock',
                'orderedList', 'bulletList', 'taskList', 'listItem', 'hardBreak', 'image', 'mention', 'opaque', 'opaqueBlock',
            ].forEach((type) => expect(nodeTypes.has(type)).toBe(true));
            [
                'bold', 'italic', 'underline', 'strike', 'code', 'link', 'textColor', 'highlight', 'textStyle',
            ].forEach((type) => expect(markTypes.has(type)).toBe(true));
            expect(state.nestedList).toBe(true);
            expect(state.orderedStart).toBe(true);
            expect(state.checkedItem).toBe(true);
            ['src', 'alt', 'title', 'width', 'height'].forEach((name) => expect(state.imageAttrs.has(name)).toBe(true));
        }
    });

    it('makes device evidence phase-scoped, nonempty, and attached to real traversal surfaces', () => {
        const iosInstrumentation = readSource('ios/Viewer/PreparedProseInstrumentation.swift');
        const androidInstrumentation = readSource('android/src/main/java/com/apollohg/editor/viewer/PreparedProseInstrumentation.kt');
        const iosHarness = readSource('ios/Tests/NativePerformanceTests.swift');
        const androidDevice = readSource('android/src/androidTest/java/com/apollohg/editor/NativeDevicePerformanceTest.kt');
        const androidHarness = readSource('android/src/sharedTest/java/com/apollohg/editor/NativePerformanceSupport.kt');

        [iosInstrumentation, androidInstrumentation].forEach((source) => {
            expect(source).toContain('TraversalPhase');
            expect(source).toContain('combined');
            expect(source).toContain('drawCount');
            expect(source).toContain('phaseSamples');
        });
        [iosHarness, androidHarness].forEach((source) => {
            expect(source).toContain('requireNonEmpty');
            expect(source).toContain('drawCount');
        });
        expect(iosHarness).toContain('viewerFrameNanos');
        expect(androidHarness).toContain('warmViewerFrames');
        expect(androidDevice).toContain('import org.json.JSONObject');
        expect(androidDevice).toContain('activity.setContentView');
        expect(androidDevice).toContain('instrumentation.waitForIdleSync()');
        expect(androidDevice).toContain('harness.traverseFromInstrumentationThread');
        expect(androidHarness).toContain('CountDownLatch');
        expect(androidHarness).toContain('Choreographer.getInstance().postFrameCallback');
        expect(androidHarness).toContain('RecyclerView');
    });

    it('releases and reacquires direct and Fabric ownership without evicting mounted artifacts', () => {
        const iosView = readSource('ios/ProseViewerView.swift');
        const androidView = readSource('android/src/main/java/com/apollohg/editor/ProseViewerView.kt');
        const iosCache = readSource('ios/Viewer/PreparedProseLayoutCache.swift');
        const androidCache = readSource('android/src/main/java/com/apollohg/editor/viewer/PreparedProseLayoutCache.kt');
        const androidMeasure = androidView.slice(
            androidView.indexOf('override fun onMeasure('),
            androidView.indexOf('override fun onLayout('),
        );
        const androidDirectOwnership = androidView.slice(
            androidView.indexOf('private fun registerDirectMountedArtifactIfAttached('),
            androidView.indexOf('private fun releaseDirectMountedArtifact()'),
        );
        const androidAttach = androidView.slice(
            androidView.indexOf('override fun onAttachedToWindow()'),
            androidView.indexOf('override fun onConfigurationChanged('),
        );

        expect(iosView).toContain('releaseDirectMounted(preparedInstrumentationOwner)');
        expect(iosView).toContain('registerDirectMounted(preparedInstrumentationOwner, layout: layout)');
        expect(androidView).toContain('override fun onAttachedToWindow()');
        expect(androidView).toContain('override fun onDetachedFromWindow()');
        expect(androidView).toContain('releaseDirectMounted(preparedInstrumentationOwner)');
        expect(androidView).toContain('registerDirectMounted(preparedInstrumentationOwner, artifact)');
        // A View may be measured but never attached, so measurement must leave
        // the artifact evictable when memory pressure clears unmounted layouts.
        expect(androidMeasure).toContain('registerDirectMountedArtifactIfAttached(artifact)');
        expect(androidDirectOwnership).toContain('if (!isAttachedToWindow) return');
        expect(androidAttach).toContain('preparedArtifact?.let(::registerDirectMountedArtifactIfAttached)');
        expect(androidCache).toContain('fun removeAllUnmounted() = synchronized(lock) { completed.clear(); mountIndex.clear();');
        expect(iosCache).toContain('retireUnownedPublicationKeysLocked');
        expect(iosCache).toContain('releaseLease');
        expect(iosCache).toContain('publishOwnerBytesLocked');
        expect(androidCache).toContain('retireUnownedPublicationsLocked');
        expect(androidCache).toContain('releaseLease');
        expect(androidCache).toContain('publishOwnersLocked');
    });

    it('compiles Android draw instrumentation out of release drawing', () => {
        const drawing = readSource('android/src/main/java/com/apollohg/editor/viewer/PreparedProseDrawingView.kt');
        const debugTiming = readSource('android/src/debug/java/com/apollohg/editor/viewer/PreparedProseDrawInstrumentation.kt');
        const releaseTiming = readSource('android/src/release/java/com/apollohg/editor/viewer/PreparedProseDrawInstrumentation.kt');

        expect(drawing).toContain('recordPreparedProseDraw {');
        expect(drawing).not.toContain('PreparedProseInstrumentation.');
        expect(debugTiming).toContain('PreparedProseInstrumentation.now()');
        expect(debugTiming).toContain('PreparedProseInstrumentation.drew(started, draw())');
        expect(releaseTiming).toContain('internal inline fun recordPreparedProseDraw');
        expect(releaseTiming).toContain('draw()');
        expect(releaseTiming).not.toContain('PreparedProseInstrumentation');
        expect(releaseTiming).not.toContain('BuildConfig');
    });

    it('ships iOS instrumentation in the actual test target exactly once', () => {
        const projectYml = readSource('ios-tests/project.yml');
        const pbxproj = readSource('ios-tests/NativeEditorTests.xcodeproj/project.pbxproj');
        expect(projectYml).toContain('../ios/Viewer/PreparedProseInstrumentation.swift');
        expect((pbxproj.match(/PreparedProseInstrumentation\.swift/g) ?? [])).toHaveLength(4);
    });

    it('shares one complete corpus schema and image policy with every benchmark surface', () => {
        const configuration = JSON.parse(readSource('scripts/tests/prepared-prose-benchmark-config.json')) as {
            configuration: { initialization: { type: string }; schema: { nodes: Array<{ name: string; attrs?: Record<string, unknown> }>; marks: Array<{ name: string; attrs?: Record<string, unknown> }> } };
            imageLoadingPolicy: Record<string, number>;
        };
        const nodeNames = new Set(configuration.configuration.schema.nodes.map(({ name }) => name));
        const markNames = new Set(configuration.configuration.schema.marks.map(({ name }) => name));
        expect(configuration.configuration.initialization).toEqual({ type: 'localEmpty' });
        [
            'doc', 'paragraph', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote', 'codeBlock',
            'orderedList', 'bulletList', 'taskList', 'listItem', 'image', 'hardBreak', 'mention', 'opaque', 'opaqueBlock',
        ].forEach((name) => expect(nodeNames.has(name)).toBe(true));
        ['bold', 'italic', 'underline', 'strike', 'code', 'link', 'textColor', 'highlight', 'textStyle']
            .forEach((name) => expect(markNames.has(name)).toBe(true));
        expect(configuration.configuration.schema.nodes.find(({ name }) => name === 'image')?.attrs)
            .toMatchObject({ src: {}, alt: { default: null }, title: { default: null }, width: { default: null }, height: { default: null } });
        expect(configuration.configuration.schema.marks.find(({ name }) => name === 'textStyle')?.attrs)
            .toMatchObject({ fontFamily: {}, fontSize: {} });
        expect(configuration.imageLoadingPolicy).toMatchObject({ maxConcurrentRequests: 2, maxPendingRequests: 64 });

        const iosHarness = readSource('ios/Tests/NativePerformanceTests.swift');
        const androidHarness = readSource('android/src/sharedTest/java/com/apollohg/editor/NativePerformanceSupport.kt');
        const example = readSource('example/App.tsx');
        [iosHarness, androidHarness, example].forEach((source) => {
            expect(source).toContain('prepared-prose-benchmark-config');
        });
        expect(iosHarness).not.toContain('configuration: .init(imagesEnabled:');
        expect(androidHarness).not.toContain('ProseViewerConfiguration(configJson = "{}"');
        expect(example).toContain('schema={preparedViewerConfiguration.configuration.schema}');
        expect(example).toContain('imageLoadingPolicy={preparedViewerConfiguration.imageLoadingPolicy}');
    });

    it('keeps native and FlatList phases separate from reset, measurement, and export', () => {
        const iosInstrumentation = readSource('ios/Viewer/PreparedProseInstrumentation.swift');
        const androidInstrumentation = readSource('android/src/main/java/com/apollohg/editor/viewer/PreparedProseInstrumentation.kt');
        const iosHarness = readSource('ios/Tests/NativePerformanceTests.swift');
        const androidHarness = readSource('android/src/sharedTest/java/com/apollohg/editor/NativePerformanceSupport.kt');
        const example = readSource('example/App.tsx');
        const fixtures = JSON.parse(readSource('scripts/tests/prepared-prose-harness-static-fixtures.json')) as {
            preparation: { requirement: string };
            differingHeights: { requirement: string };
            drawEvidence: { requirement: string };
        };

        [iosInstrumentation, androidInstrumentation].forEach((source) => {
            expect(source).toContain('beginPhase');
            expect(source).toContain('endPhase');
            expect(source).toContain('completedPhases');
        });
        expect(iosHarness).toContain('viewer.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))');
        expect(iosHarness).toContain('cell.prepareAndMeasure(width: collectionView.bounds.width)');
        expect(iosHarness).toContain('testPreparedProseHarnessStaticFixtures');
        expect(iosHarness).toContain('PREPARED_PROSE_STATIC_HARNESS_FIXTURES');
        expect(iosHarness).toContain('XCTAssertGreaterThan(longHeight, shortHeight)');
        expect(iosHarness).not.toContain('height: 180');
        expect(androidHarness).toContain('PreparedProseInstrumentation.beginPhase(phase)');
        expect(androidHarness).toContain('PreparedProseInstrumentation.endPhase()');
        expect(example).toContain('preparedProseBenchmarkBeginPhase');
        expect(example).toContain('preparedProseBenchmarkEndPhase');
        expect(example).toContain('scrollToOffset({ offset, animated: false })');
        expect(example).toContain('await waitForDrawSettle()');
        expect(example).toContain('Reset is intentionally not a traversal phase');
        expect(fixtures.preparation.requirement).toContain('prepare once');
        expect(fixtures.differingHeights.requirement).toContain('taller');
        expect(fixtures.drawEvidence.requirement).toContain('final settled frame');
    });
});
