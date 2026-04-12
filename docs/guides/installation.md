[Back to docs index](../README.md)

# Installation

## Current Packaging Status

The package is published, but it currently requires Expo Modules in the host app.

The minimum tested Expo version is SDK 54. Earlier Expo versions may work, but they are not currently validated by this repository.

## Required Peer Dependencies

The consuming app is expected to install:

- `expo`
- `react`
- `react-native`
- `@expo/vector-icons`

These are peer dependencies because the package integrates directly with the host app’s React Native and Expo runtime.

## Expo Requirement

This package is not a plain React Native native module. It is built on Expo Modules and currently expects an Expo-based native setup.

Supported integration shapes today:

- Expo app with a development build
- bare React Native app with Expo Modules configured

Unsupported shape:

- Expo Go

## Local Repository Setup

From the repository root:

```sh
npm install
npm --prefix example install
```

## Example App Setup

The repository includes an Expo SDK 54 example app under [`example`](../../example).

Prebuild the native projects (required after a fresh clone or any native dependency change):

```sh
npm run example:prebuild
```

This runs `expo prebuild --clean` inside the example app, generating the `ios/` and `android/` directories with all native module wiring in place.

Start Metro:

```sh
npm run example:start
```

Run iOS:

```sh
npm run example:ios
```

Run Android:

```sh
npm run example:android
```

## iOS Notes

Prebuild runs `pod install` automatically. You only need to run it manually if you modify the Podfile or add iOS-specific dependencies outside of prebuild.

This package contains native code, so use a development build rather than Expo Go.

## Consumer-App Shape

The expected consumer setup looks like:

```sh
npm install @apollohg/react-native-prose-editor@0.5.0
npx expo install expo react react-native @expo/vector-icons
npx expo prebuild
```

The `expo prebuild` step is required because this package includes native iOS and Android code. It generates the native projects with the correct module wiring.

For in-repo development, the linked example app remains the fastest way to verify native changes locally.

## Common Setup Checks

If the editor does not build correctly, check:

- you have run `expo prebuild` (or `npx expo prebuild`) to generate the native projects
- the host app has the required peer dependencies installed directly
- there is only one version of each native dependency in the app
- iOS pods are installed (prebuild does this automatically)
- you are not trying to run the native module inside Expo Go

## Related Docs

- [Getting Started](./getting-started.md)
- [Development Workflow](../development/workflow.md)
