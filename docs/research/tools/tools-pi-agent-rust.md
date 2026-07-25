# pi_agent_rust tools

Reference: `Dicklesworthstone/pi_agent_rust` at `9fcdb65`.

## Architecture

The Rust port defines one async, object-safe `Tool` trait with `name`, `label`, `description`, JSON Schema `parameters`, `execute`, and `effects`. `ToolRegistry` constructs enabled built-ins and resolves calls by name. Calls return `ToolOutput { content: Vec<ContentBlock>, details, is_error }`; long operations may publish `ToolUpdate`s.

`ToolEffects` declares read, write, append, network, and process effects. Unknown declarations fail closed to write; only compatible non-barrier effects are scheduled together. Paths are normalized and constrained to cwd. Runtime failures use `Error::tool(name, message)`. Read-only outputs can be cached against file/directory fingerprints. Model output defaults to 2,000 lines/1 MiB; large source output may spill to redacted, session-scoped artifacts while retaining a bounded preview. Limits also protect input file/image sizes and subprocess output.

Implementation: `src/tools.rs` (trait, registry, all eight implementations, truncation/cache/artifacts), `src/agent.rs` (tool-call scheduling), and `src/error.rs`.

## Tools

### `read`

- **Schema/usage:** required `path`; optional one-based `offset`/`limit`. Text and common image formats are accepted.
- **Dispatch/execution:** cwd-scoped async file I/O; identifies and optionally resizes/re-encodes images, while text is range-read and cacheable.
- **Output/errors:** text/image content blocks plus metadata; bounded preview and artifact reference for oversized source. Rejects directories, files above 100 MiB, invalid ranges/encoding/images, or scope/I/O failures through `Error::tool`.
- **Implementation:** `src/tools.rs` (`ReadTool`, image helpers, artifact helpers).

### `bash`

- **Schema/usage:** required `command`; optional positive timeout.
- **Dispatch/execution:** prepares the configured shell, spawns a process group, reads stdout/stderr concurrently, emits incremental updates, and cancels/terminates the tree with a grace period.
- **Output/errors:** merged bounded output, exit/timeout/cancellation details, and spillover artifact for large output. Setup/spawn/reader failures are tool errors; non-zero exit and timeout preserve output and status.
- **Implementation:** `src/tools.rs` (`BashTool`, process and cancellation helpers).

### `edit`

- **Schema/usage:** required `path` plus `edits[]` of exact `oldText`/`newText` replacements.
- **Dispatch/execution:** cwd-scoped, size-bounded read; validates all edits and applies an atomic in-memory replacement set before writing.
- **Output/errors:** success summary and structured edit details. Empty, absent, duplicate, ambiguous, overlapping, or no-op matches fail without partial writes; scope, size, encoding, and I/O failures are tool errors.
- **Implementation:** `src/tools.rs` (`EditTool`, replacement validators).

### `write`

- **Schema/usage:** required `path` and full `content`.
- **Dispatch/execution:** cwd-scoped create/replace, parent-directory creation, 100 MiB content limit.
- **Output/errors:** concise write confirmation/details. Scope violations, oversized content, parent creation, and write failures return `Error::tool`.
- **Implementation:** `src/tools.rs` (`WriteTool`).

### `grep`

- **Schema/usage:** required regex `pattern`; optional `path`, glob, and bounded `limit`.
- **Dispatch/execution:** runs ripgrep in cwd scope, reads streams concurrently, supports cancellation, and caches results against recursive directory fingerprints.
- **Output/errors:** `path:line:match` text; defaults to 100 matches and truncates individual lines at 500 characters. No match is success; invalid regex/path, timeout/cancel, ripgrep startup, and unexpected exit are tool errors.
- **Implementation:** `src/tools.rs` (`GrepTool`).

### `find`

- **Schema/usage:** required pattern; optional root and limit.
- **Dispatch/execution:** runs `fd` with ignore-aware discovery under a 60-second bound; result is cacheable against directory fingerprints.
- **Output/errors:** sorted/bounded path list, default maximum 1,000. No match succeeds; invalid scope/root, timeout/cancel, startup, reader, or non-search exit fails as a named tool error.
- **Implementation:** `src/tools.rs` (`FindTool`).

### `ls`

- **Schema/usage:** optional directory path and result limit.
- **Dispatch/execution:** cwd-scoped directory scan, metadata lookup, deterministic ordering, and directory suffixes; caches against entry fingerprints.
- **Output/errors:** default maximum 500 entries and hard scan ceiling 20,000, with explicit omission/truncation metadata. Non-directories and directory/entry metadata failures are tool errors.
- **Implementation:** `src/tools.rs` (`LsTool`).

### `hashline_edit`

- **Schema/usage:** required `path` and non-empty edits using line anchors encoded as line-number/content hashes; operations support replacement, insertion, and deletion over anchored ranges.
- **Dispatch/execution:** reads a cwd-scoped text file, verifies every anchor against current content, resolves edits, applies them in a stable order, and writes once.
- **Output/errors:** reports applied operations and new hashline context. Stale/malformed/ambiguous anchors, overlapping edits, invalid ranges, binary/large files, and I/O failures abort the whole edit with `Error::tool`.
- **Implementation:** `src/tools.rs` (`HashlineEditTool`, hashline parsing/hash helpers).
