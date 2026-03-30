[Back to docs index](../README.md)

# Architecture

## High-Level Model

The editor is split across four layers:

1. React Native public API in `src`
2. Native iOS and Android editor views
3. Native render and position bridges
4. Rust document core

The design goal is to keep document semantics in Rust and keep platform rendering and input behavior in native code.

## React Native Layer

This layer is responsible for:

- props and ref API
- controlled and uncontrolled content flow
- toolbar configuration
- theme serialization
- bridging editor updates back into React callbacks

## Native Layer

The native iOS and Android views are responsible for:

- text input integration with the platform IME
- focus and blur behavior
- mapping native selection to Rust scalar positions
- rendering styled content
- dispatching commands to the Rust core

The native layer should not own document truth. It should reflect and manipulate the Rust editor state.

## Rust Core

The Rust core owns:

- schema validation
- document model
- transforms and step application
- selection normalization
- history
- active formatting state
- HTML and JSON serialization
- render tree generation

This is the source of truth for editor semantics.

## Useful Boundary

When deciding where a fix belongs:

| Problem Type | Likely Home |
| --- | --- |
| document meaning, transforms, schema validity, history | Rust |
| cursor geometry, platform layout, IME behavior, native rendering | iOS or Android native code |
| public configuration, composition, app integration | React Native layer |

For the rationale behind the less obvious API choices, see [Design Decisions](./design-decisions.md).
