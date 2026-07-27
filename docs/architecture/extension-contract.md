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

## Installation and discovery

Installed extensions live in `extensions/<id>/` beside the Wren executable. `extension.toml` registers each extension without loading its library:

```toml
id = "read"
generation = "<build-id>"
library = "generations/<build-id>/wren_read_extension.dll"
mode = "auto"
```

The ID must match the installation directory. The library path is relative to that directory and cannot escape it. Generation-specific library paths allow later builds to be installed without overwriting code that Windows may have mapped into a running process.

`cargo install-wren` installs the harness and its bundled read and write extensions into Cargo's binary directory.

## Configuration and loading

Wren reads `%USERPROFILE%/.wren/config.toml`. `WREN_HOME` changes the directory containing `config.toml` for tests and development. Missing configuration is equivalent to an empty file.

Installed extensions select either `auto` or `manual` loading. User configuration can override that mode and explicitly request manual extensions:

```toml
[extensions]
load = ["database"]

[extensions.read]
mode = "manual"
```

Auto and explicitly requested extensions load before command dispatch. Loading validates the build fingerprint, initializes the extension, checks that its initialized name matches its installed ID, and rejects duplicate tool names. Libraries remain loaded for the lifetime of the process. Reloading and unloading are separate lifecycle work.

A loaded tool is invoked without a library argument. Windows PowerShell 5.1 requires the JSON quotes to be escaped for native argument passing:

```powershell
wren tool read --args '{\"path\":\"Cargo.toml\"}'
```

PowerShell 7 accepts the JSON directly:

```powershell
wren tool read --args '{"path":"Cargo.toml"}'
```

Successful tool text is written unchanged to stdout. Configuration, discovery, loading, JSON, dispatch, and tool failures are written to stderr and produce a non-zero exit status. The former `--extension` path is not supported.

Dependency resolution, runtime compilation, unloading, reload behavior, stdin arguments, and schema-derived CLI flags remain outside the contract.
