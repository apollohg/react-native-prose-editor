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
});
