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
});
