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
});
