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
    it('injects the raw host JNA JAR into JVM Test tasks without contaminating Android resolution', () => {
        const androidBuild = readSource('android/build.gradle');

        expect(androidBuild).toContain('api "net.java.dev.jna:jna:5.18.1@aar"');
        expect(androidBuild).toContain('hostTestJna {');
        expect(androidBuild).toContain('canBeConsumed = false');
        expect(androidBuild).toContain('canBeResolved = true');
        expect(androidBuild).toContain('transitive = false');
        expect(androidBuild).toContain('hostTestJna "net.java.dev.jna:jna:5.18.1@jar"');
        expect(androidBuild).toContain("name.endsWith('UnitTestRuntimeClasspath')");
        expect(androidBuild).toContain("exclude group: 'net.java.dev.jna', module: 'jna'");
        expect(androidBuild).toContain('def hostTestJnaClasspath = files(configurations.hostTestJna)');
        expect(androidBuild).toContain('tasks.withType(Test).configureEach {');
        expect(androidBuild).toContain("inputs.files(hostTestJnaClasspath).withPropertyName('hostTestJnaClasspath')");
        expect(androidBuild).toContain('classpath = hostTestJnaClasspath.plus(classpath)');
        expect(androidBuild).not.toContain('testRuntimeOnly files(configurations.hostTestJna)');
        expect(androidBuild).not.toContain('testRuntimeOnly "net.java.dev.jna:jna:5.18.1"');
    });

    it('keeps the hard cutover validator manifest-driven across exports and native project membership', () => {
        const validator = readSource('scripts/tests/validate-prepared-prose-viewer-cutover.mjs');
        const manifest = JSON.parse(readSource('scripts/package-abi-manifest.json')) as {
            preparedProseViewer: { removedPaths: string[] };
        };

        expect(validator).toContain("const exists = (path) => existsSync(resolve(root, path));");
        expect(validator).toContain('for (const path of manifest.removedPaths)');
        expect(validator).toContain("'src/index.ts'");
        expect(validator).toContain("'ios-tests/project.yml'");
        expect(validator).toContain("'ios-tests/NativeEditorTests.xcodeproj/project.pbxproj'");
        expect(manifest.preparedProseViewer.removedPaths).toEqual([
            'android/src/main/java/com/apollohg/editor/NativeProseViewerExpoView.kt',
            'android/src/test/java/com/apollohg/editor/NativeProseViewerExpoViewTest.kt',
            'android/src/test/java/com/apollohg/editor/ProseViewerViewTest.kt',
            'ios/NativeProseViewerExpoView.swift',
            'ios/Tests/ProseViewerViewTests.swift',
            'src/heightCache.ts',
            'src/__tests__/heightCache.test.ts',
        ]);
    });

    it('keeps an iOS Fabric artifact generation intact for link-permission-only updates', () => {
        const source = readSource('ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm');
        const generationComparator = methodBody(source, 'bool HasEquivalentGenerationProps(');
        const updateProps = methodBody(source, '- (void)updateProps:');

        expect(generationComparator).not.toContain('enableLinkTaps');
        expect(updateProps).toContain('const BOOL generationChanged =');
        expect(updateProps).toContain(
            'if (generationChanged) {\n    [self beginNewGenerationTerminatingCurrentLease:NO];\n  }'
        );
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
        const androidBeginImages = android.slice(
            android.indexOf('fun beginImages('),
            android.indexOf('/** Phase one of Fabric image setup: no artifact is required yet. */'),
        );
        const androidMeasure = android.slice(android.indexOf('override fun measure('), android.indexOf('private fun update('));

        expect(iosProps.indexOf('_viewerProps = nextProps;')).toBeLessThan(iosProps.indexOf('[self beginSemanticImageGenerationIfPossible];'));
        expect(iosState.indexOf('_viewerState = nextState;')).toBeLessThan(iosState.indexOf('[self beginSemanticImageGenerationIfPossible];'));
        expect(iosDrawing).toContain('@objc(beginSemanticImageGeneration:)');
        expect(androidUpdate.indexOf('state.mutation()')).toBeLessThan(androidUpdate.indexOf('state.beginSemanticImageGeneration(view)'));
        expect(androidUpdate.indexOf('state.beginSemanticImageGeneration(view)')).toBeLessThan(androidUpdate.indexOf('reconcile(view, state)'));
        expect(androidMeasure).toContain('val leaseHandle = FabricLeaseHandleBridge.currentHandle()');
        const androidDeclinesWithoutLease = androidMeasure.slice(
            androidMeasure.indexOf('if (surface != null && leaseHandle <= 0L) {'),
            androidMeasure.indexOf('val request = requestFrom(props, state, leaseHandle)'),
        );
        expect(androidDeclinesWithoutLease).toContain('return YogaMeasureOutput.make(0f, 0f)');
        expect(androidMeasure).toContain('val request = requestFrom(props, state, leaseHandle)');
        expect(androidMeasure.indexOf('val request = requestFrom(props, state, leaseHandle)')).toBeLessThan(
            androidMeasure.indexOf('PreparedProseLayoutRegistry.shared.measure(')
        );
        expect(androidMeasure).toContain('fabricSurface = surface, fabricLeaseHandle = leaseHandle');
        expect(androidBeginImages).toContain('attachmentRevisions.admit(artifact.imageAttachments.size)');
        expect(androidBeginImages).toContain('fontEnvironment.activate()');
        expect(androidBeginImages).toContain('imagePipeline.begin(request.semanticGenerationIdentity');
        expect(androidBeginImages).not.toContain('attachmentRevisions.beginSemanticGeneration');
        expect(androidBeginImages).not.toContain('imagePipeline.cancel()');
    });

    it('drops Android Fabric sidecars through the exact persisted generation after view tags mutate', () => {
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
        expect(adopt).toContain('if (previousSidecar != null && previousSidecar != next)');
        expect(adopt).toContain('PreparedProseLayoutRegistry.shared.releaseFabricGeneration(previousSidecar)');
        expect(release).toContain('releaseSidecarOwnership()');
        expect(releaseSidecar).toContain('val sidecar = sidecarGeneration ?: return');
        expect(releaseSidecar).toContain('sidecarGeneration = null');
        expect(releaseSidecar).toContain('PreparedProseLayoutRegistry.shared.releaseFabricGeneration(sidecar)');
    });

    it('preserves iOS Fabric ownership across detach and terminates the recorded lease on recycle or dealloc', () => {
        const ios = readSource('ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm');
        const state = readSource('common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerState.h');
        const shadow = readSource('common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerShadowNode.cpp');
        const detach = methodBody(ios, '- (void)didMoveToSuperview');
        const recycle = methodBody(ios, '- (void)prepareForRecycle');
        const dealloc = methodBody(ios, '- (void)dealloc');
        const beginGeneration = methodBody(ios, '- (void)beginNewGenerationTerminatingCurrentLease:');
        const releaseOwnership = methodBody(ios, '- (void)releaseFabricOwnershipTerminatingLease:');
        const install = methodBody(ios, '- (void)installMeasuredArtifactIfAttached');

        expect(detach).toContain('[_drawingView cancelConfiguredImages];');
        expect(detach).not.toContain('[self releaseAllFabricOwnership];');
        expect(recycle).toContain('[self releaseAllFabricOwnership];');
        expect(dealloc).toContain('[self releaseAllFabricOwnership];');
        expect(beginGeneration).toContain('if (terminal) {\n    [self releaseFabricOwnershipTerminatingLease:YES];\n  }');
        expect(releaseOwnership).toContain('const auto stateLeaseHandle = LeaseHandle(_viewerState);');
        expect(releaseOwnership).toContain('DeactivateLease(_viewerState, stateLeaseHandle);');
        expect(releaseOwnership).toContain('releaseFabricLeaseSurfaceId:_ownedSurfaceId');
        expect(releaseOwnership).toContain('componentTag:_ownedComponentTag');
        expect(releaseOwnership).toContain('leaseHandle:leaseHandle');
        expect(releaseOwnership).not.toContain('self.tag');
        expect(state).toContain('SurfaceId surfaceId{0};');
        expect(state).toContain('Tag componentTag{0};');
        expect(shadow).toContain('state.surfaceId = family->getSurfaceId();');
        expect(shadow).toContain('state.componentTag = family->getTag();');
        expect(install).toContain('const auto surfaceId = SurfaceIdFromState(_viewerState);');
        expect(install).toContain('const auto componentTag = ComponentTagFromState(_viewerState);');
        expect(ios).not.toContain('SurfaceIdForComponentView');
        expect(install).not.toContain('self.tag');
    });

    it('keeps every very-long literal independently complete and preserves the exact traversal contract', () => {
        const corpus = JSON.parse(readSource('scripts/tests/viewer-performance-corpus.json')) as {
            documents: Array<{ id: string; category: string; contentJSON: unknown }>;
            coldTraversal: string[];
            warmTraversal: string[];
            warmWindows: Array<{ id: string; primeIds: string[]; warmIds: string[] }>;
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
        expect(corpus.warmWindows).toHaveLength(27);
        expect(new Set(corpus.warmWindows.map(({ id }) => id)).size).toBe(27);
        expect(corpus.warmWindows.flatMap(({ primeIds }) => primeIds).sort())
            .toEqual(corpus.documents.map(({ id }) => id).sort());
        for (const window of corpus.warmWindows) {
            expect(window.warmIds).toEqual(window.primeIds);
        }
        expect(corpus.warmWindows.filter(({ id }) => id.startsWith('very-long-'))).toEqual([
            { id: 'very-long-01', primeIds: ['very-long-0001'], warmIds: ['very-long-0001'] },
            { id: 'very-long-02', primeIds: ['very-long-0002'], warmIds: ['very-long-0002'] },
            { id: 'very-long-03', primeIds: ['very-long-0003'], warmIds: ['very-long-0003'] },
            { id: 'very-long-04', primeIds: ['very-long-0004'], warmIds: ['very-long-0004'] },
            { id: 'very-long-05', primeIds: ['very-long-0005'], warmIds: ['very-long-0005'] },
        ]);

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
        [iosInstrumentation, androidInstrumentation].forEach((source) => {
            expect(source).toContain('rawFrameDeltasNanos');
            expect(source).toContain('viewerCausedDelayedIntervals');
            expect(source).toContain('imageRequestCount');
            expect(source).toContain('imageMetadataCount');
            expect(source).toContain('imageDecodeCount');
        });
        [iosHarness, androidHarness].forEach((source) => {
            expect(source).toContain('rawFrameDeltasNanos');
            expect(source).toContain('viewerCausedDelayedIntervals');
            expect(source).toContain('preResetSnapshot');
            expect(source).toContain('postResetSnapshot');
        });
        expect(androidDevice).toContain('import org.json.JSONObject');
        expect(androidDevice).toContain('activity.setContentView');
        expect(androidDevice).toContain('instrumentation.waitForIdleSync()');
        expect(androidDevice).toContain('harness.traverseWindows(\n                instrumentation,');
        expect(androidHarness).toContain('CountDownLatch');
        expect(androidHarness).toContain('val completion = CountDownLatch(1)');
        expect(androidHarness).toContain('instrumentation.runOnMainSync {');
        expect(androidHarness).toContain('completion.countDown()');
        expect(androidHarness).toContain('completion.await(10, TimeUnit.SECONDS)');
        expect(androidHarness).not.toContain('traverseFromInstrumentationThread');
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
        const androidReset = androidCache.slice(
            androidCache.indexOf('fun removeAllUnmounted()'),
            androidCache.indexOf('internal val completedCountForTesting'),
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
        expect(androidReset).toContain('completed.clear()');
        expect(androidReset).toContain('mountIndex.clear()');
        expect(androidReset).toContain('pendingLeases.clear()');
        expect(androidReset).toContain('retireUnownedPublicationsLocked()');
        expect(androidReset).toContain('publishOwnersLocked()');
        expect(androidReset).not.toContain('mountedLeases.clear()');
        expect(iosCache).toContain('retireUnownedPublicationKeysLocked');
        expect(iosCache).toContain('releaseLease');
        expect(iosCache).toContain('publishOwnerBytesLocked');
        expect(iosCache).toContain('pendingLeases');
        expect(iosCache).toContain('mountedLeases');
        expect(iosCache).toContain('removePendingLeaseLocked(leaseKey)');
        expect(androidCache).toContain('retireUnownedPublicationsLocked');
        expect(androidCache).toContain('releaseLease');
        expect(androidCache).toContain('publishOwnersLocked');
    });

    it('compiles Android draw instrumentation out of release drawing', () => {
        const drawing = readSource('android/src/main/java/com/apollohg/editor/viewer/PreparedProseDrawingView.kt');
        const debugTiming = readSource('android/src/debug/java/com/apollohg/editor/viewer/PreparedProseDrawInstrumentation.kt');
        const releaseTiming = readSource('android/src/release/java/com/apollohg/editor/viewer/PreparedProseDrawInstrumentation.kt');
        const onDraw = drawing.slice(
            drawing.indexOf('override fun onDraw(canvas: Canvas)'),
            drawing.indexOf('private fun drawBackground'),
        );

        expect(onDraw).toContain('recordPreparedProseDraw {');
        expect(onDraw).not.toContain('PreparedProseInstrumentation.');
        expect(debugTiming).toContain('PreparedProseInstrumentation.now()');
        expect(debugTiming).toContain('PreparedProseInstrumentation.drew(started, draw())');
        expect(releaseTiming).toContain('internal inline fun recordPreparedProseDraw');
        expect(releaseTiming).toContain('draw()');
        expect(releaseTiming).not.toContain('PreparedProseInstrumentation');
        expect(releaseTiming).not.toContain('BuildConfig');
    });

    it('ships iOS instrumentation through one file reference and one NativeEditorTests sources membership', () => {
        const projectYml = readSource('ios-tests/project.yml');
        const pbxproj = readSource('ios-tests/NativeEditorTests.xcodeproj/project.pbxproj');
        const fileReferenceMatches = [
            ...pbxproj.matchAll(
                /([A-F0-9]{24}) \/\* PreparedProseInstrumentation\.swift \*\/ = \{isa = PBXFileReference;[^\n]*path = PreparedProseInstrumentation\.swift;/g
            ),
        ];
        expect(fileReferenceMatches).toHaveLength(1);
        const fileReferenceId = fileReferenceMatches[0]![1]!;
        const buildFileMatches = [
            ...pbxproj.matchAll(
                new RegExp(
                    `([A-F0-9]{24}) /\\* PreparedProseInstrumentation\\.swift in Sources \\*/ = \\{isa = PBXBuildFile; fileRef = ${fileReferenceId}`,
                    'g'
                )
            ),
        ];
        expect(buildFileMatches).toHaveLength(1);
        const buildFileId = buildFileMatches[0]![1]!;
        const nativeTargetSectionStart = pbxproj.indexOf('/* Begin PBXNativeTarget section */');
        const nativeTestsTargetStart = pbxproj.indexOf(
            ' /* NativeEditorTests */ = {',
            nativeTargetSectionStart
        );
        expect(nativeTestsTargetStart).toBeGreaterThan(-1);
        const nativeTestsTarget = pbxproj.slice(
            nativeTestsTargetStart,
            pbxproj.indexOf('\n\t\t};', nativeTestsTargetStart)
        );
        const sourcesBuildPhaseMatches = [
            ...nativeTestsTarget.matchAll(/([A-F0-9]{24}) \/\* Sources \*\//g),
        ];
        expect(sourcesBuildPhaseMatches).toHaveLength(1);
        const sourcesBuildPhaseId = sourcesBuildPhaseMatches[0]![1]!;
        const sourcesBuildPhase = pbxproj.slice(
            pbxproj.indexOf(`${sourcesBuildPhaseId} /* Sources */ = {`),
            pbxproj.indexOf('/* End PBXSourcesBuildPhase section */'),
        );

        expect(projectYml.match(/^\s*- path: \.\.\/ios\/Viewer\/PreparedProseInstrumentation\.swift$/gm)).toHaveLength(1);
        expect(sourcesBuildPhase.match(new RegExp(`${buildFileId} /\\* PreparedProseInstrumentation\\.swift in Sources \\*/,`, 'g'))).toHaveLength(1);
    });

    it('routes the iPhone 13 prepared-prose release gate through its dedicated launch environment', () => {
        const packageJson = JSON.parse(readSource('package.json')) as { scripts: Record<string, string> };
        const runner = readSource('scripts/run-ios-tests.sh');
        const projectYml = readSource('ios-tests/project.yml');
        const ordinaryScheme = readSource('ios-tests/NativeEditorTests.xcodeproj/xcshareddata/xcschemes/NativeEditorTests.xcscheme');
        const preparedSchemePath = path.join(
            REPO_ROOT,
            'ios-tests/NativeEditorTests.xcodeproj/xcshareddata/xcschemes/NativeEditorPreparedProsePerformance.xcscheme'
        );

        expect(fs.existsSync(preparedSchemePath)).toBe(true);
        const preparedScheme = fs.readFileSync(preparedSchemePath, 'utf8');
        const preparedTestAction = preparedScheme.slice(
            preparedScheme.indexOf('<TestAction'),
            preparedScheme.indexOf('</TestAction>') + '</TestAction>'.length
        );
        const preparedLaunchAction = preparedScheme.slice(
            preparedScheme.indexOf('<LaunchAction'),
            preparedScheme.indexOf('</LaunchAction>') + '</LaunchAction>'.length
        );

        expect(packageJson.scripts['test:ios:device:performance']).toBe(
            'NATIVE_EDITOR_IOS_TEST_SCHEME=NativeEditorPreparedProsePerformance bash ./scripts/run-ios-on-device.sh -only-testing:NativeEditorTests/NativePerformanceTests/testPerformance_preparedProseCorpusGates_iPhone13'
        );
        expect(packageJson.scripts['test:ios:performance']).toBe(
            'bash ./scripts/run-ios-tests.sh -only-testing:NativeEditorTests/NativePerformanceTests'
        );
        expect(packageJson.scripts['test:ios']).toBe('bash ./scripts/run-ios-tests.sh');
        expect(runner).toContain('scheme="${NATIVE_EDITOR_IOS_TEST_SCHEME:-NativeEditorTests}"');
        expect(runner).toContain('-scheme "$scheme"');
        expect(ordinaryScheme).not.toContain('PREPARED_PROSE_DEVICE_BENCHMARK');
        expect(preparedTestAction).toContain('shouldUseLaunchSchemeArgsEnv = "YES"');
        expect(preparedLaunchAction).toContain('<EnvironmentVariable\n            key = "PREPARED_PROSE_DEVICE_BENCHMARK"\n            value = "1"\n            isEnabled = "YES">');
        expect(projectYml).toContain('NativeEditorPreparedProsePerformance:');
        expect(projectYml).toContain('PREPARED_PROSE_DEVICE_BENCHMARK: "1"');
    });

    it('prints the prepared-prose benchmark export before gating the same local export', () => {
        const performanceTests = readSource('ios/Tests/NativePerformanceTests.swift');
        const benchmark = performanceTests.slice(
            performanceTests.indexOf('func testPerformance_preparedProseCorpusGates_iPhone13() throws {'),
            performanceTests.indexOf('\n    /// Fixture-only device contract')
        );
        const localExport = 'let benchmarkExport = PreparedProseInstrumentation.exportJSON()';
        const diagnostic = 'print("[PreparedProseBenchmarkExport]\\(benchmarkExport)")';
        const gate = 'exportJSON: benchmarkExport,';
        const exportCalls = benchmark.match(/PreparedProseInstrumentation\.exportJSON\(\)/g) ?? [];

        expect(benchmark).toContain(localExport);
        expect(benchmark).toContain(diagnostic);
        expect(benchmark).toContain(gate);
        expect(exportCalls).toHaveLength(1);
        expect(benchmark.indexOf(localExport)).toBeLessThan(benchmark.indexOf(diagnostic));
        expect(benchmark.indexOf(diagnostic)).toBeLessThan(benchmark.indexOf(gate));
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
        const iosCollectionHarnessStart = iosHarness.lastIndexOf('private final class PreparedProseCollectionHarness');
        const iosPerformanceGatesStart = iosHarness.lastIndexOf('private enum PreparedProsePerformanceGates');
        expect(iosCollectionHarnessStart).toBeGreaterThanOrEqual(0);
        expect(iosPerformanceGatesStart).toBeGreaterThan(iosCollectionHarnessStart);
        const iosCollectionHarness = iosHarness.slice(iosCollectionHarnessStart, iosPerformanceGatesStart);

        [iosInstrumentation, androidInstrumentation].forEach((source) => {
            expect(source).toContain('beginPhase');
            expect(source).toContain('endPhase');
            expect(source).toContain('completedPhases');
        });
        expect(iosCollectionHarness).toContain('func traverseWindows(');
        expect(iosCollectionHarness).toContain('override func preferredLayoutAttributesFitting(_ attributes: UICollectionViewLayoutAttributes)');
        expect(iosCollectionHarness).toContain('let width = max(1, attributes.size.width)');
        expect(iosCollectionHarness).toContain('viewer.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))');
        ['measurementView', 'prepareAndMeasure', 'RunLoop.main.run', 'scrollToItem(at:'].forEach((removedPath) =>
            expect(iosCollectionHarness).not.toContain(removedPath)
        );
        expect(iosHarness).toContain('testPreparedProseHarnessStaticFixtures');
        expect(iosHarness).toContain('PREPARED_PROSE_STATIC_HARNESS_FIXTURES');
        expect(iosHarness).toContain('XCTAssertGreaterThan(longHeight, shortHeight)');
        expect(iosHarness).not.toContain('height: 180');
        expect(androidHarness).toContain(
            'val evidencePhase = if (active.direction == Direction.WARM && active.phase == PreparedProseInstrumentation.TraversalPhase.COLD) {\n' +
            '            PreparedProseInstrumentation.TraversalPhase.WARM\n' +
            '        } else {\n' +
            '            active.phase\n' +
            '        }'
        );
        expect(androidHarness).toContain('PreparedProseInstrumentation.beginPhase(evidencePhase)');
        expect(androidHarness).toContain('PreparedProseInstrumentation.endPhase()');
        expect(example).toContain('preparedProseBenchmarkBeginPhase');
        expect(example).toContain('preparedProseBenchmarkEndPhase');
        expect(example).toContain('preparedViewerCorpus.warmWindows');
        expect(example).toContain('scrollToEnd({ animated: true })');
        expect(example).toContain('scrollToIndex({ index: 0, animated: true })');
        expect(example).toContain('dispatchOffsetY');
        expect(example).toContain('latestNativeContentOffsetYRef');
        expect(example).toContain('onScroll={handleScroll}');
        [
            'scrollToOffset',
            'waitForDrawSettle',
            'onContentSizeChange',
            'contentHeightRef',
            'measureInWindow',
            'heightCache',
            'containerWidth',
            'getItemLayout',
        ].forEach((removedPath) => expect(example).not.toContain(removedPath));
        expect(example).toContain('Reset is intentionally not a traversal phase');
        expect(fixtures.preparation.requirement).toContain('prepare once');
        expect(fixtures.differingHeights.requirement).toContain('taller');
        expect(fixtures.drawEvidence.requirement).toContain('final settled frame');
    });

    it('loads Android benchmark fixtures from the instrumentation package and requires the Expo bridge', () => {
        const androidDevice = readSource('android/src/androidTest/java/com/apollohg/editor/NativeDevicePerformanceTest.kt');
        const example = readSource('example/App.tsx');

        expect(androidDevice).toContain('private val testContext: Context = instrumentation.context');
        expect(androidDevice).toContain('private val targetContext: Context = instrumentation.targetContext');
        expect(androidDevice).toContain('testContext.assets.open("viewer-performance-corpus.json")');
        expect(androidDevice).toContain('PreparedProseBenchmarkConfiguration.load(testContext)');
        expect(androidDevice).not.toContain('ApplicationProvider.getApplicationContext');

        // `expo-modules-core` is nested under `expo/node_modules` in the example
        // install, so a bare specifier does not resolve there. `expo` re-exports
        // `requireNativeModule`; the contract is that it is not RN NativeModules.
        expect(example).toContain("import { requireNativeModule } from 'expo';");
        expect(example).toContain("requireNativeModule<PreparedProseBenchmarkBridge>('NativeEditor')");
        expect(example).not.toContain('NativeModules');
        expect(example).not.toContain('preparedProseBenchmarkBridge?.');
        expect(example).not.toContain("?? '{}'");
    });
});
