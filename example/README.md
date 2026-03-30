# Example App

This is a local Expo SDK 54 app for developing and manually testing the native editor package in this repository.

The package currently requires Expo Modules, and Expo SDK 54 is the minimum version tested in this repo.

It is intended for:

- focus and keyboard dismissal checks
- toolbar interactions
- list and line break behavior
- theme iteration under Fast Refresh
- native iOS and Android verification

## Install

From the repository root:

```sh
cd example
npm install
```

## Prebuild

This package contains native code, so you need to generate the native projects before building:

```sh
npm run prebuild
```

This runs `expo prebuild --clean`, creating the `ios/` and `android/` directories. Re-run this after any native dependency change or Expo config update.

## Run

From the repository root:

```sh
npm run example:ios
npm run example:android
```

Or directly inside `example/`:

```sh
npm run ios
npm run android
```

## Notes

- This package contains native code — use a development build, not Expo Go.
- The example app depends on the local package via `file:..`.
- If you change native code or Rust bindings, rebuild the app after updating the package binaries.
- If the native build fails after pulling new changes, try running prebuild again.
- The example screen intentionally uses app-level keyboard avoidance around the outer `ScrollView`. That is the expected integration for a screen-level form or playground.
- On Android, the example also adds `keyboardVerticalOffset={60}` so `KeyboardAvoidingView` accounts for the built-in native keyboard toolbar, not just the IME.
- The editor still manages its own internal caret visibility and fixed-height scrolling. Screen-level keyboard avoidance and native editor viewport handling are complementary, not interchangeable.
