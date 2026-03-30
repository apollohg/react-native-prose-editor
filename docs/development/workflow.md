[Back to docs index](../README.md)

# Development Workflow

## Install

```sh
npm install
npm --prefix example install
```

## TypeScript

```sh
npm run typecheck
npm --prefix example run typecheck
```

## Run the Example App

```sh
npm run example:start
npm run example:ios
npm run example:android
```

## Rebuild Rust Artifacts

Full rebuild (iOS + Android + bindings):

```sh
npm run build:rust
```

Platform-specific:

```sh
npm run build:rust:ios
npm run build:rust:android
```

## Tests

### TypeScript Unit Tests

```sh
npm test
```

### Rust Core Tests

```sh
cargo test --manifest-path rust/editor-core/Cargo.toml
```

### Android Unit Tests

```sh
npm run android:test
```

This runs the Robolectric-based unit tests for the Android native module. You can also compile-check Android Kotlin without running tests:

```sh
npm run android:compile
```

### iOS XCTest

```sh
xcodebuild test \
  -workspace ios-tests/NativeEditorTests.xcworkspace \
  -scheme NativeEditorTests \
  -configuration Debug \
  -destination 'platform=iOS Simulator,name=iPhone 16'
```

Adjust the simulator destination to match an installed device on your machine.

If you change [ios-tests/project.yml](../../ios-tests/project.yml), regenerate the Xcode project before running CocoaPods or tests:

```sh
cd ios-tests
xcodegen generate
pod install
```

## Prebuild the Example App

If you need to regenerate the native projects (e.g. after changing Expo config or adding native dependencies):

```sh
npm run example:prebuild
```

## When to Rebuild

Rebuild Rust outputs when you change:

- Rust core logic
- the UniFFI UDL surface
- generated Swift or Kotlin bindings

Rebuild native apps when you change:

- iOS native editor code
- Android native editor code
- Expo module wiring

## Tips

- The example app is the fastest way to validate focus, keyboard, toolbar, and theming behavior.
- The iOS XCTest target is useful for native regressions that are too specific for the example app alone.
- Android unit tests run under Robolectric and do not require a device or emulator.
