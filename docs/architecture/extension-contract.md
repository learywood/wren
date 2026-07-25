# Extension contract

Wren features are trusted Rust extensions loaded as native dynamic libraries. Extensions run in the harness process with the harness's privileges. They are not a security or fault-isolation boundary.

## Build compatibility

Wren and its extensions must be built from source with the same pinned Rust toolchain, compilation target, profile, panic strategy, and extension API revision. Wren does not support arbitrary prebuilt extension binaries.

Each library exports a stable build-fingerprint function. The loader calls only this function before comparing the extension fingerprint with its own. A match permits Wren to use the native Rust contract; a mismatch rejects the library before any Rust value crosses the boundary. Changes to the native contract increment its API revision and rebuild extensions. The tool-capable contract is API revision 2.

## Native contract

An extension depends on the `wren-extension` crate, implements its `Extension` trait, and uses `export_extension!` to expose the required symbols. The macro keeps symbol and ownership plumbing out of extension implementations.

The loader creates one extension instance and calls `Extension::initialize` once. Initialization returns the extension's non-empty name. Indexed `Extension::tool` access exposes model-callable capabilities without making every extension a tool.

A `Tool` provides a name, description, JSON Schema, and synchronous invocation. Wren validates registration metadata, parses invocation arguments as a JSON object, and passes that value unchanged to the selected tool. Invocation receives the harness working directory and returns model-visible text, optional structured JSON details, or a structured error kind and message. Calls across the native boundary must not panic.

Tool references are borrowed only while the extension is loaded; Wren copies registration names for dispatch rather than retaining self-referential borrows. Extension allocations are destroyed by code from the library that created them. Wren retains the library until its extension instance has been destroyed.

## Loading

The loader accepts one explicit dynamic-library path through `wren --extension <path>`. It validates the build fingerprint, constructs the extension, initializes it, and reports its name.

A tool can be invoked through the same production loader:

```text
wren --extension <path> tool <name> --args <json>
```

Successful tool text is written unchanged to stdout. Loading, JSON, dispatch, and tool failures are written to stderr and produce a non-zero exit status.

Discovery, installation, dependency resolution, runtime compilation, unloading, reload behavior, stdin arguments, and schema-derived CLI flags remain outside the contract.
