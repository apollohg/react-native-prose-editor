import assert from "node:assert/strict";
import { cpSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, relative } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const validator = join(repoRoot, "scripts", "validate-packed-package.sh");
const workDir = mkdtempSync(join(tmpdir(), "native-editor-packed-package-fixtures-"));
const failures = [];

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
  if (process.env.VALIDATE_PACKED_PACKAGE_FIXTURE === "wrong-native-checksum") {
    runWrongNativeChecksumFixture();
  } else if (process.env.VALIDATE_PACKED_PACKAGE_FIXTURE === "duplicate-ios-checksum") {
    runDuplicateIosChecksumFixture();
  } else {
    const baseline = makeFixture("baseline");
    expectPass("baseline ABI", run("--validate-abi-root", baseline));
    expectPass("baseline copied artifacts", run("--validate-copies", repoRoot, baseline));

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
    /legacy UniFFI function symbol: editor_create/,
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
  replace(
    join(linklessPod, "ios/ReactNativeProseEditor.podspec"),
    "s.vendored_frameworks = 'EditorCore.xcframework'",
    "# fixture intentionally omits the linked EditorCore.xcframework",
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
