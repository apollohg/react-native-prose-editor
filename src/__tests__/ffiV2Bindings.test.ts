// ─── FFI v2 production bindings contract ───────────────────────
// The checked-in UniFFI bindings are the production ABI: all 29
// `editor_v2_*` functions plus `editor_core_version`, and zero legacy
// symbols (the deleted legacy editor/collaboration runtime).
// These tests only assert the content of the checked-in artifacts — they
// never rebuild the Rust library (rust/generate-bindings.sh regenerates
// and verifies these files). An optional `nm` check against an already
// built dylib runs only when FFI_V2_BINDINGS_CHECK_DYLIB=1 is set.

import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

const REPO_ROOT = path.resolve(__dirname, '..', '..');

const V2_EXPORTS = [
    'editor_v2_create',
    'editor_v2_destroy',
    'editor_v2_get_state',
    'editor_v2_get_document_json',
    'editor_v2_get_document_html',
    'editor_v2_get_content_snapshot',
    'editor_v2_replace_document',
    'editor_v2_apply_input',
    'editor_v2_apply_command',
    'editor_v2_apply_local_api',
    'editor_v2_set_selection',
    'editor_v2_undo',
    'editor_v2_redo',
    'editor_v2_collaboration_begin_connect',
    'editor_v2_collaboration_socket_open',
    'editor_v2_collaboration_receive',
    'editor_v2_collaboration_socket_close',
    'editor_v2_collaboration_take_outbound',
    'editor_v2_collaboration_set_awareness',
    'editor_v2_collaboration_peers',
    'editor_v2_collaboration_tick',
    'editor_v2_collaboration_detach',
    'editor_v2_collaboration_reattach',
    'editor_v2_snapshot_export',
    'editor_v2_snapshot_restore',
    'editor_v2_render_update',
    'editor_v2_resolve_scalar_selection',
    'editor_v2_doc_to_scalar',
    'editor_v2_scalar_to_doc',
];

const ALL_EXPORTS = [...V2_EXPORTS, 'editor_core_version'];
const ALLOWED_FN_SYMBOLS = new Set(ALL_EXPORTS);

const SWIFT_HEADERS = [
    'rust/bindings/swift/editor_coreFFI.h',
    'ios/editor_coreFFI/editor_coreFFI.h',
];
const SWIFT_SOURCES = [
    'rust/bindings/swift/editor_core.swift',
    'ios/Generated_editor_core.swift',
];
const KOTLIN_SOURCES = ['rust/bindings/kotlin/uniffi/editor_core/editor_core.kt'];
const MODULEMAPS = [
    'rust/bindings/swift/editor_coreFFI.modulemap',
    'ios/editor_coreFFI/module.modulemap',
];
const ALL_ARTIFACTS = [
    ...SWIFT_HEADERS,
    ...SWIFT_SOURCES,
    ...KOTLIN_SOURCES,
    ...MODULEMAPS,
];

function camelCase(symbol: string): string {
    return symbol.replace(/_([a-z0-9])/g, (_match, letter: string) => letter.toUpperCase());
}

function readArtifact(relativePath: string): string {
    const absolute = path.join(REPO_ROOT, relativePath);
    expect(fs.existsSync(absolute)).toBe(true);
    return fs.readFileSync(absolute, 'utf8');
}

/** Every fn symbol referenced by an artifact must be part of the v2 ABI. */
function expectNoLegacySymbols(contents: string): void {
    expect(contents).not.toMatch(/collaboration_session|collaborationSession/);
    const fnReferences = contents.match(/uniffi_editor_core_fn_func_[a-z0-9_]+/g) ?? [];
    for (const reference of new Set(fnReferences)) {
        const symbol = reference.slice('uniffi_editor_core_fn_func_'.length);
        expect(ALLOWED_FN_SYMBOLS.has(symbol)).toBe(true);
    }
}

describe('ffi v2 production bindings', () => {
    it('ships all 29 v2 functions plus editor_core_version in the C headers (fn + checksum)', () => {
        for (const header of SWIFT_HEADERS) {
            const contents = readArtifact(header);
            for (const symbol of ALL_EXPORTS) {
                expect(contents).toContain(`uniffi_editor_core_fn_func_${symbol}`);
                expect(contents).toContain(`uniffi_editor_core_checksum_func_${symbol}`);
            }
            expectNoLegacySymbols(contents);
        }
    });

    it('exposes all 29 v2 functions plus editorCoreVersion in the Swift bindings', () => {
        for (const swift of SWIFT_SOURCES) {
            const contents = readArtifact(swift);
            for (const symbol of ALL_EXPORTS) {
                expect(contents).toContain(`${camelCase(symbol)}(`);
            }
            expectNoLegacySymbols(contents);
        }
    });

    it('exposes all 29 v2 functions plus editorCoreVersion in the Kotlin bindings', () => {
        for (const kotlin of KOTLIN_SOURCES) {
            const contents = readArtifact(kotlin);
            for (const symbol of ALL_EXPORTS) {
                expect(contents).toContain(camelCase(symbol));
            }
            expectNoLegacySymbols(contents);
        }
    });

    it('ships modulemaps that expose the v2 FFI header and nothing legacy', () => {
        for (const modulemap of MODULEMAPS) {
            const contents = readArtifact(modulemap);
            expect(contents).toContain('header "editor_coreFFI.h"');
            expectNoLegacySymbols(contents);
        }
    });

    it('contains no legacy symbol in any checked-in binding artifact', () => {
        for (const artifact of ALL_ARTIFACTS) {
            expectNoLegacySymbols(readArtifact(artifact));
        }
    });

    it('keeps the ios package bindings byte-identical to the generated rust bindings', () => {
        expect(readArtifact('ios/Generated_editor_core.swift')).toBe(
            readArtifact('rust/bindings/swift/editor_core.swift')
        );
        expect(readArtifact('ios/editor_coreFFI/editor_coreFFI.h')).toBe(
            readArtifact('rust/bindings/swift/editor_coreFFI.h')
        );
        expect(readArtifact('ios/editor_coreFFI/module.modulemap')).toBe(
            readArtifact('rust/bindings/swift/editor_coreFFI.modulemap')
        );
    });

    // Opt-in only: inspects an already-built dylib, never builds one.
    const checkDylib = process.env.FFI_V2_BINDINGS_CHECK_DYLIB === '1' ? it : it.skip;
    checkDylib('exports exactly the 29 v2 symbols from an existing release dylib', () => {
        const dylib = path.join(
            process.env.CARGO_TARGET_DIR ?? path.join(REPO_ROOT, 'rust', 'editor-core', 'target'),
            'release',
            'libeditor_core.dylib'
        );
        const nmOutput = execFileSync('nm', ['-gU', dylib], {
            encoding: 'utf8',
            maxBuffer: 64 * 1024 * 1024,
        });
        for (const symbol of ALL_EXPORTS) {
            expect(nmOutput).toContain(`uniffi_editor_core_fn_func_${symbol}`);
        }
        const v2Symbols = nmOutput
            .split('\n')
            .filter((line) => line.includes('uniffi_editor_core_fn_func_editor_v2_'));
        expect(v2Symbols).toHaveLength(V2_EXPORTS.length);
        expectNoLegacySymbols(nmOutput);
    });
});
