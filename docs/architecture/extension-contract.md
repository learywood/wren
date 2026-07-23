# Extension contract

Wren features are trusted Rust extensions loaded as native dynamic libraries. Extensions run in the harness process with the harness's privileges. They are not a security or fault-isolation boundary.

## Build compatibility

Wren and its extensions must be built from source with the same pinned Rust toolchain, compilation target, profile, panic strategy, and extension API revision. Wren does not support arbitrary prebuilt extension binaries.

Each library exports a stable build-fingerprint function. The loader calls only this function before comparing the extension fingerprint with its own. A match permits Wren to use the native Rust contract; a mismatch rejects the library before any Rust value crosses the boundary. Changes to the native contract must increment its API revision and rebuild extensions.

## Native contract

An extension depends on the `wren-extension` crate, implements its `Extension` trait, and uses `export_extension!` to expose the required symbols. The macro keeps symbol and ownership plumbing out of extension implementations.

The loader creates one extension instance and calls `Extension::initialize` once. Initialization returns the extension's non-empty name. Calls across the native boundary must not panic.

Extension allocations are destroyed by code from the library that created them. Wren retains the library until its extension instance has been destroyed.

## Loading

The initial loader accepts one explicit dynamic-library path through `wren --extension <path>`. It validates the build fingerprint, constructs the extension, initializes it, and reports its name.

Discovery, installation, dependency resolution, runtime compilation, unloading, and reload behavior are outside the initial contract. Capability-specific interfaces will be added only when an implemented extension demonstrates their need.
