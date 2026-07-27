import assert from "node:assert/strict";
import { cpSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, relative } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const validator = join(repoRoot, "scripts", "validate-packed-package.sh");
const checksumValidator = join(repoRoot, "scripts", "validate-uniffi-checksum-values.rb");
const checksumManifest = join(repoRoot, "scripts", "package-abi-manifest.json");
const validatorSource = readFileSync(validator, "utf8");
const workDir = mkdtempSync(join(tmpdir(), "native-editor-packed-package-fixtures-"));
const failures = [];

assert.match(
  validatorSource,
  /npm install --ignore-scripts --no-audit --no-fund --offline --package-lock=false --legacy-peer-deps/,
  "packed iOS consumer must install its generated tarball through npm without lifecycle scripts or network access",
);
assert.match(
  validatorSource,
  /"@apollohg\/react-native-prose-editor": "file:\$tarball_path"/,
  "packed iOS consumer package.json must declare the generated tarball as its editor dependency",
);
assert.match(
  validatorSource,
  /require\.resolve\('@apollohg\/react-native-prose-editor\/package\.json'\)/,
  "packed iOS consumer must prove Node resolves the installed editor package",
);
assert.match(
  validatorSource,
  /iOS packed consumer resolved editor package outside consumer node_modules/,
  "packed iOS consumer must reject an editor package resolved outside its node_modules",
);
assert.match(
  validatorSource,
  /iOS packed consumer resolved editor package from repository or extraction staging/,
  "packed iOS consumer must reject a resolved package realpath pointing at the repository or extracted staging tree",
);
assert.doesNotMatch(
  validatorSource,
  /cp -R "\$root" "\$packed_editor_dir"/,
  "packed iOS consumer must not manually copy the extracted package into node_modules",
);
assert.doesNotMatch(
  validatorSource,
  /"@apollohg\/react-native-prose-editor": "file:\.\/node_modules\/@apollohg\/react-native-prose-editor"/,
  "packed iOS consumer must not self-reference a manually populated node_modules dependency",
);

function copyPath(sourceRoot, destinationRoot, path) {
  const source = join(sourceRoot, path);
  const destination = join(destinationRoot, path);
  mkdirSync(dirname(destination), { recursive: true });
  cpSync(source, destination, { recursive: true });
}

function makeFixture(name) {
  const fixture = join(workDir, name);
  mkdirSync(fixture, { recursive: true });
  for (const path of [
    "package.json",
    "LICENSE",
    "ReactNativeProseEditor.podspec",
    "ios",
    "android",
    "rust/android",
    "rust/ios",
    "rust/bindings/kotlin",
    "rust/bindings/swift",
  ]) {
    copyPath(repoRoot, fixture, path);
  }
  return fixture;
}

function makePackageEntriesFixture(name, legacyText = "") {
  const fixture = join(workDir, name);
  mkdirSync(join(fixture, "dist"), { recursive: true });
  writeFileSync(join(fixture, "dist/index.js"), `export const ready = true;\n${legacyText}`);
  writeFileSync(
    join(fixture, "dist/index.d.ts"),
    [
      "export declare class NativeEditorBoundaryError extends Error {}",
      "export interface NativeCollaborationTransportConfig { url: string; connect: boolean }",
      "export interface NativeCollaborationTransportEvent { editorId: string }",
      "export interface ResourceLimits { resourceLimits?: unknown }",
      "export interface ImagePolicy { requestTimeoutMs?: number }",
      legacyText,
    ].join("\n"),
  );
  return fixture;
}

function replace(path, from, to) {
  const contents = readFileSync(path, "utf8");
  assert.ok(contents.includes(from), `fixture setup could not find ${from} in ${relative(repoRoot, path)}`);
  writeFileSync(path, contents.replace(from, to));
}

function replaceBytes(path, from, to) {
  const contents = readFileSync(path);
  const offset = contents.indexOf(from);
  assert.notEqual(offset, -1, `fixture setup could not find native bytes in ${relative(repoRoot, path)}`);
  assert.equal(contents.indexOf(from, offset + 1), -1, `fixture setup found multiple native byte sequences in ${relative(repoRoot, path)}`);
  to.copy(contents, offset);
  writeFileSync(path, contents);
}

function run(...args) {
  const result = spawnSync("bash", [validator, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
    env: { ...process.env, NODE_COMPILE_CACHE: join(workDir, "node-compile-cache") },
  });
  return { status: result.status, output: `${result.stdout}\n${result.stderr}` };
}

function runFixtureCommand(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: repoRoot, encoding: "utf8", ...options });
  assert.equal(result.status, 0, `fixture command failed: ${command} ${args.join(" ")}\n${result.stdout}\n${result.stderr}`);
  return result.stdout;
}

function runNativeChecksumValidator(...args) {
  const result = spawnSync(
    "ruby",
    [checksumValidator, "--manifest", checksumManifest, "--label", "native parser bounds fixture", ...args],
    { cwd: repoRoot, encoding: "utf8" },
  );
  return { status: result.status, output: `${result.stdout}\n${result.stderr}` };
}

function expectPass(name, result) {
  if (result.status !== 0) {
    failures.push(`${name}: expected success, got ${result.status}\n${result.output}`);
  }
}

function expectFailure(name, result, expected) {
  if (result.status === 0) {
    failures.push(`${name}: validator accepted the broken fixture`);
  } else if (!expected.test(result.output)) {
    failures.push(`${name}: expected ${expected}, got\n${result.output}`);
  }
}

function bsdArchiveMember(name, contents) {
  const nameBytes = Buffer.from(name, "utf8");
  const memberSize = nameBytes.length + contents.length;
  const header = Buffer.alloc(60, 0x20);
  header.write(`#1/${nameBytes.length}`, 0, "ascii");
  header.write(String(memberSize), 48, "ascii");
  header.write("`\n", 58, "ascii");
  const member = Buffer.concat([header, nameBytes, contents]);
  return memberSize % 2 === 0 ? member : Buffer.concat([member, Buffer.from("\n")]);
}

function elfHeader({ sectionCount, programCount, elfClass = 2, type = 3, headerSize = elfClass === 1 ? 52 : 64, machine = elfClass === 1 ? 40 : 183 }) {
  const contents = Buffer.alloc(256);
  Buffer.from([0x7f, 0x45, 0x4c, 0x46, elfClass, 1, 1]).copy(contents);
  contents.writeUInt16LE(type, 16);
  contents.writeUInt16LE(machine, 18);
  contents.writeUInt32LE(1, 20);
  if (elfClass === 1) {
    contents.writeUInt32LE(64, 32);
    contents.writeUInt16LE(headerSize, 40);
    contents.writeUInt16LE(32, 42);
    contents.writeUInt16LE(programCount, 44);
    contents.writeUInt16LE(40, 46);
    contents.writeUInt16LE(sectionCount, 48);
  } else {
    contents.writeBigUInt64LE(64n, 40);
    contents.writeUInt16LE(headerSize, 52);
    contents.writeUInt16LE(56, 54);
    contents.writeUInt16LE(programCount, 56);
    contents.writeUInt16LE(64, 58);
    contents.writeUInt16LE(sectionCount, 60);
  }
  return contents;
}

function machoObject({ fileType = 1, commandCount = 0 }) {
  const contents = Buffer.alloc(32);
  contents.writeUInt32LE(0xfeedfacf, 0);
  contents.writeUInt32LE(0x0100000c, 4);
  contents.writeUInt32LE(fileType, 12);
  contents.writeUInt32LE(commandCount, 16);
  return contents;
}

function runNativeParserBoundsFixtures() {
  const archivePath = join(workDir, "too-many-archive-members.a");
  const archive = [Buffer.from("!<arch>\n")];
  for (let index = 0; index < 4097; index += 1) archive.push(bsdArchiveMember("__.SYMDEF", Buffer.alloc(0)));
  writeFileSync(archivePath, Buffer.concat(archive));
  expectFailure(
    "archive member count bound",
    runNativeChecksumValidator("--macho-archive", "arm64", archivePath),
    /native parser bounds fixture has too many archive members: 4097 exceeds 4096/,
  );

  const elfSectionCountPath = join(workDir, "too-many-elf-sections.so");
  writeFileSync(elfSectionCountPath, elfHeader({ sectionCount: 4097, programCount: 0 }));
  expectFailure(
    "ELF section count bound",
    runNativeChecksumValidator("--elf", "arm64-v8a", elfSectionCountPath),
    /native parser bounds fixture has too many ELF section headers: 4097 exceeds 4096/,
  );

  const elfProgramCountPath = join(workDir, "too-many-elf-program-headers.so");
  writeFileSync(elfProgramCountPath, elfHeader({ sectionCount: 1, programCount: 4097 }));
  expectFailure(
    "ELF program header count bound",
    runNativeChecksumValidator("--elf", "arm64-v8a", elfProgramCountPath),
    /native parser bounds fixture has too many ELF program headers: 4097 exceeds 4096/,
  );

  const elfTypePath = join(workDir, "wrong-elf-type.so");
  writeFileSync(elfTypePath, elfHeader({ sectionCount: 1, programCount: 1, type: 2 }));
  expectFailure(
    "ELF type identity",
    runNativeChecksumValidator("--elf", "arm64-v8a", elfTypePath),
    /native parser bounds fixture has the wrong ELF file type/,
  );

  const elfHeaderSizePath = join(workDir, "wrong-elf-header-size.so");
  writeFileSync(elfHeaderSizePath, elfHeader({ sectionCount: 1, programCount: 1, headerSize: 52 }));
  expectFailure(
    "ELF header size identity",
    runNativeChecksumValidator("--elf", "arm64-v8a", elfHeaderSizePath),
    /native parser bounds fixture has the wrong ELF header size/,
  );

  const elf32HeaderSizePath = join(workDir, "wrong-elf32-header-size.so");
  const elf32Header = elfHeader({ sectionCount: 1, programCount: 1, elfClass: 1, headerSize: 64 });
  elf32Header.writeUInt16LE(52, 52);
  writeFileSync(elf32HeaderSizePath, elf32Header);
  expectFailure(
    "ELF32 header size identity",
    runNativeChecksumValidator("--elf", "armeabi-v7a", elf32HeaderSizePath),
    /native parser bounds fixture has the wrong ELF header size/,
  );

  const elfDynamicSymbolsPath = join(workDir, "too-many-elf-dynamic-symbols.so");
  const elfDynamicSymbols = elfHeader({ sectionCount: 3, programCount: 1 });
  elfDynamicSymbols.writeUInt32LE(11, 128 + 4);
  elfDynamicSymbols.writeBigUInt64LE(24n * 1_000_001n, 128 + 32);
  elfDynamicSymbols.writeUInt32LE(2, 128 + 40);
  elfDynamicSymbols.writeBigUInt64LE(24n, 128 + 56);
  writeFileSync(elfDynamicSymbolsPath, elfDynamicSymbols);
  expectFailure(
    "ELF dynamic symbol count bound",
    runNativeChecksumValidator("--elf", "arm64-v8a", elfDynamicSymbolsPath),
    /native parser bounds fixture has too many ELF dynamic symbols: 1000001 exceeds 1000000/,
  );

  const machoCommandCountPath = join(workDir, "too-many-macho-commands.a");
  writeFileSync(machoCommandCountPath, Buffer.concat([Buffer.from("!<arch>\n"), bsdArchiveMember("bounds.o", machoObject({ commandCount: 4097 }))]));
  expectFailure(
    "Mach-O load command count bound",
    runNativeChecksumValidator("--macho-archive", "arm64", machoCommandCountPath),
    /native parser bounds fixture object bounds\.o \(member 1\) has too many Mach-O load commands: 4097 exceeds 4096/,
  );

  const machoFileTypePath = join(workDir, "wrong-macho-filetype.a");
  writeFileSync(machoFileTypePath, Buffer.concat([Buffer.from("!<arch>\n"), bsdArchiveMember("bounds.o", machoObject({ fileType: 2 }))]));
  expectFailure(
    "Mach-O file type identity",
    runNativeChecksumValidator("--macho-archive", "arm64", machoFileTypePath),
    /native parser bounds fixture object bounds\.o \(member 1\) has the wrong Mach-O file type/,
  );
}

function runWrongNativeChecksumFixture() {
  const wrongNativeChecksum = makeFixture("wrong-native-checksum");
  replaceBytes(
    join(wrongNativeChecksum, "rust/android/x86_64/libeditor_core.so"),
    Buffer.from([0x66, 0xb8, 0x94, 0x38, 0xc3]),
    Buffer.from([0x66, 0xb8, 0x01, 0x00, 0xc3]),
  );
  expectPass("synchronized native copies", run("--validate-copies", wrongNativeChecksum, wrongNativeChecksum));
  expectFailure(
    "wrong native checksum value",
    run("--validate-android-library", join(wrongNativeChecksum, "rust/android/x86_64/libeditor_core.so"), "x86_64"),
    /Android x86_64 library checksum mismatch for editor_v2_apply_command: expected 14484, found 1/,
  );
}

function runDuplicateIosChecksumFixture() {
  const duplicateIosChecksum = makeFixture("duplicate-ios-checksum");
  const archive = join(duplicateIosChecksum, "ios/EditorCore.xcframework/ios-arm64/libeditor_core.a");
  const extractedObjects = join(workDir, "duplicate-ios-checksum-objects");
  mkdirSync(extractedObjects, { recursive: true });
  runFixtureCommand("ar", ["-x", archive], { cwd: extractedObjects });

  const checksumObject = readdirSync(extractedObjects).find((name) => {
    if (!name.endsWith(".o")) return false;
    const result = spawnSync("nm", ["-gU", join(extractedObjects, name)], { encoding: "utf8" });
    return result.status === 0 && result.stdout.includes("_uniffi_editor_core_checksum_func_editor_v2_apply_command");
  });
  assert.ok(checksumObject, "fixture setup could not find the iOS checksum-defining object");

  runFixtureCommand("ar", ["-q", archive, join(extractedObjects, checksumObject)]);
  runFixtureCommand("ranlib", [archive]);
  expectFailure(
    "duplicate iOS checksum under the original member name",
    run("--validate-xcframework", join(duplicateIosChecksum, "ios/EditorCore.xcframework")),
    /iOS device arm64 archive has duplicate checksum symbol editor_v2_apply_command/,
  );
}

try {
  if (process.env.VALIDATE_PACKED_PACKAGE_FIXTURE === "native-parser-bounds") {
    runNativeParserBoundsFixtures();
  } else if (process.env.VALIDATE_PACKED_PACKAGE_FIXTURE === "wrong-native-checksum") {
    runWrongNativeChecksumFixture();
  } else if (process.env.VALIDATE_PACKED_PACKAGE_FIXTURE === "duplicate-ios-checksum") {
    runDuplicateIosChecksumFixture();
  } else {
    const baseline = makeFixture("baseline");
    expectPass("baseline ABI", run("--validate-abi-root", baseline));
    expectPass("baseline copied artifacts", run("--validate-copies", repoRoot, baseline));
    expectPass(
      "native transport package entries",
      run("--validate-package-entries", makePackageEntriesFixture("package-entries")),
    );
    expectFailure(
      "legacy JavaScript socket package entry",
      run(
        "--validate-package-entries",
        makePackageEntriesFixture("legacy-package-entry", "export const createWebSocket = () => {};"),
      ),
      /obsolete collaboration API createWebSocket/,
    );
    runNativeParserBoundsFixtures();

    const missingFunction = makeFixture("missing-function");
    replace(
      join(missingFunction, "ios/editor_coreFFI/editor_coreFFI.h"),
      "uniffi_editor_core_fn_func_editor_v2_undo",
      "uniffi_editor_core_fn_func_editor_v2_undo_removed",
    );
    expectFailure(
      "missing v2 function",
      run("--validate-abi-root", missingFunction),
      /missing expected function symbol: editor_v2_undo/,
    );

    const legacyFunction = makeFixture("legacy-function");
    writeFileSync(
      join(legacyFunction, "ios/editor_coreFFI/editor_coreFFI.h"),
      "\nRustBuffer uniffi_editor_core_fn_func_editor_create(RustCallStatus *_Nonnull out_status);\n",
      { flag: "a" },
    );
    expectFailure(
      "legacy function",
      run("--validate-abi-root", legacyFunction),
      /unexpected UniFFI function symbol: editor_create/,
    );

    const missingViewerMethod = makeFixture("missing-viewer-method");
    replace(
      join(missingViewerMethod, "ios/editor_coreFFI/editor_coreFFI.h"),
      "uniffi_editor_core_fn_method_viewercompileddocument_semantic_key",
      "uniffi_editor_core_fn_method_viewercompileddocument_semantic_key_removed",
    );
    expectFailure(
      "missing ViewerCompiledDocument method",
      run("--validate-abi-root", missingViewerMethod),
      /object methods is missing expected function symbol: viewercompileddocument_semantic_key/,
    );

    const wrongChecksum = makeFixture("wrong-checksum");
    replace(
      join(wrongChecksum, "ios/Generated_editor_core.swift"),
      "uniffi_editor_core_checksum_func_editor_v2_apply_command() != 14484",
      "uniffi_editor_core_checksum_func_editor_v2_apply_command() != 1",
    );
    expectFailure(
      "wrong Swift checksum",
      run("--validate-abi-root", wrongChecksum),
      /Swift checksum mismatch for editor_v2_apply_command/,
    );

    const staleSwift = makeFixture("stale-swift");
    replace(join(staleSwift, "ios/Generated_editor_core.swift"), "editorV2Create", "editorV2CreateStale");
    expectFailure(
      "stale Swift binding",
      run("--validate-copies", repoRoot, staleSwift),
      /copy mismatch: ios\/Generated_editor_core\.swift/,
    );

    const staleKotlin = makeFixture("stale-kotlin");
    replace(
      join(staleKotlin, "rust/bindings/kotlin/uniffi/editor_core/editor_core.kt"),
      "fun `editorV2Create`(",
      "fun `editorV2CreateStale`(",
    );
    expectFailure(
      "stale Kotlin binding",
      run("--validate-copies", repoRoot, staleKotlin),
      /copy mismatch: rust\/bindings\/kotlin\/uniffi\/editor_core\/editor_core\.kt/,
    );

    const staleIosBinary = makeFixture("stale-ios-binary");
    writeFileSync(
      join(staleIosBinary, "ios/EditorCore.xcframework/ios-arm64/libeditor_core.a"),
      Buffer.from([0]),
      { flag: "a" },
    );
    expectFailure(
      "stale iOS binary",
      run("--validate-copies", repoRoot, staleIosBinary),
      /copy mismatch: ios\/EditorCore\.xcframework\/ios-arm64\/libeditor_core\.a/,
    );

    const staleAndroidBinary = makeFixture("stale-android-binary");
    writeFileSync(
      join(staleAndroidBinary, "rust/android/arm64-v8a/libeditor_core.so"),
      Buffer.from([0]),
      { flag: "a" },
    );
    expectFailure(
      "stale Android binary",
      run("--validate-copies", repoRoot, staleAndroidBinary),
      /copy mismatch: rust\/android\/arm64-v8a\/libeditor_core\.so/,
    );

    const linklessPod = makeFixture("linkless-pod");
    const linklessPodspec = join(linklessPod, "ReactNativeProseEditor.podspec");
    replace(
      linklessPodspec,
      "s.vendored_frameworks = 'ios/EditorCore.xcframework'",
      "# fixture intentionally omits the linked ios/EditorCore.xcframework",
    );
    const linklessPodspecSource = readFileSync(linklessPodspec, "utf8");
    assert.ok(
      linklessPodspecSource.includes("# fixture intentionally omits the linked ios/EditorCore.xcframework"),
      "linkless pod fixture must replace the relocated root-podspec framework declaration",
    );
    assert.ok(
      !linklessPodspecSource.includes("s.vendored_frameworks = 'ios/EditorCore.xcframework'"),
      "linkless pod fixture must remove the relocated framework declaration",
    );
    expectFailure(
      "CocoaPods installs but cannot link",
      run("--validate-ios-consumer", linklessPod),
      /iOS consumer xcodebuild failed/,
    );

    const unpackagedAndroid = makeFixture("unpackaged-android");
    replace(
      join(unpackagedAndroid, "android/build.gradle"),
      '"${project.projectDir}/../rust/android"',
      '"${project.projectDir}/../rust/android-disabled"',
    );
    expectFailure(
      "Gradle compiles Kotlin but omits native packaging",
      run("--validate-android-consumer", unpackagedAndroid),
      /Android consumer package is missing libeditor_core\.so for arm64-v8a/,
    );
    runWrongNativeChecksumFixture();
    runDuplicateIosChecksumFixture();
  }
} finally {
  rmSync(workDir, { recursive: true, force: true });
}

if (failures.length > 0) {
  throw new Error(`validate-packed-package negative fixture failures:\n\n${failures.join("\n\n")}`);
}

console.log("Packed-package negative fixtures passed.");
