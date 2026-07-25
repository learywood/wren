# Pi tools

Reference: `earendil-works/pi` at `5bc1c2c`.

## Architecture

Pi exposes tools through `AgentTool<TParameters,TDetails>`: name, label, description, TypeBox schema, and an asynchronous `execute(toolCallId, params, signal, onUpdate)` method. Core factories accept injectable `Operations` and filesystem adapters; this separates the protocol-facing tool from local I/O and permits SSH or alternate backends. `ToolDefinition` wrappers can override descriptions and parameters without changing execution.

The agent loop validates each call against its tool schema. Calls are sequential unless parallel execution is enabled; even then, tools with `concurrency: "exclusive"` form barriers. `read`, `grep`, `find`, and `ls` are read-only; `edit` and `write` are exclusive. `bash` streams updates and handles cancellation. Text results use content blocks plus optional details and `isError`; unhandled exceptions become erroneous tool results. Shared truncation is 2,000 lines or 50 KiB, preserving the beginning or end according to tool semantics and appending a continuation hint.

Implementation: `packages/agent/src/types.ts`, `packages/agent/src/agent-loop.ts`, and `packages/coding-agent/src/core/tools/{index,tool-definition-wrapper,truncate}.ts`.

## Tools

### `read`

- **Schema/usage:** `path` required; optional one-based `offset` and `limit`. Reads text ranges or supported images.
- **Dispatch/execution:** resolves relative paths against session cwd, stats through the injected filesystem, detects image MIME by extension, and otherwise delegates line reading to operations.
- **Output/errors:** text uses numbered chunks and reports remaining-line continuation; oversized lines/chunks are bounded. Images return an image content block and metadata. Missing paths, directories, bad ranges, unsupported images, and backend failures throw tool errors.
- **Implementation:** `packages/coding-agent/src/core/tools/read.ts`.

### `bash`

- **Schema/usage:** `command` required; optional `timeout` in seconds.
- **Dispatch/execution:** invokes the operations backend with cwd, abort signal, timeout, and an update callback. The local backend selects the configured shell and terminates the process tree on timeout/abort.
- **Output/errors:** combines stdout and stderr in arrival order, streams truncated snapshots, and keeps the tail of final output. Empty success is rendered explicitly; non-zero status, timeout, or cancellation is annotated in details/text rather than losing captured output. Spawn/setup failures throw.
- **Implementation:** `packages/coding-agent/src/core/tools/bash.ts` (`BashOperations` and the local implementation are defined beside the tool).

### `edit`

- **Schema/usage:** `path` and `edits[]`; each edit contains exact `oldText` and replacement `newText`.
- **Dispatch/execution:** reads the file, normalizes line endings for matching, requires each target to be unique and non-overlapping, applies all replacements in memory, then performs one write through a per-file mutation queue.
- **Output/errors:** returns a short success message and structured diff details. Fails before writing on missing/duplicate/non-unique targets, overlap, no-op replacements, invalid file state, or write failure; optional writethrough diagnostics can accompany success.
- **Implementation:** `packages/coding-agent/src/core/tools/edit.ts`, `edit-diff.ts`, and `file-mutation-queue.ts`.

### `write`

- **Schema/usage:** required `path` and complete string `content`; creation and overwrite share one operation.
- **Dispatch/execution:** resolves the path, creates parent directories, and writes through the filesystem adapter under the per-file mutation queue.
- **Output/errors:** returns bytes/line-oriented success details and optional writethrough diagnostics. Invalid paths and directory/write failures throw; it does not provide append or conditional-create semantics.
- **Implementation:** `packages/coding-agent/src/core/tools/write.ts` and `file-mutation-queue.ts`.

### `grep`

- **Schema/usage:** required regex `pattern`; optional search `path`, file `glob`, and result `limit`.
- **Dispatch/execution:** delegates to ripgrep-backed operations with hidden/ignore behavior determined by that backend; resolves paths from cwd.
- **Output/errors:** emits `path:line: text`, bounded by count and shared byte/line truncation, with a narrowing hint when limited. No matches are a successful textual result; invalid regex, inaccessible roots, missing executable, and process failures throw.
- **Implementation:** `packages/coding-agent/src/core/tools/grep.ts` (`GrepOperations` and the local implementation are defined beside the tool).

### `find`

- **Schema/usage:** required glob-like `pattern`; optional root `path` and `limit`.
- **Dispatch/execution:** delegates filename/path discovery to the operations backend (local implementation uses `fd` where available), respecting ignore rules.
- **Output/errors:** returns relative matching paths, bounded by result count and truncation, or a successful no-match message. Invalid roots/patterns and backend failures throw.
- **Implementation:** `packages/coding-agent/src/core/tools/find.ts` (`FindOperations` and the local implementation are defined beside the tool).

### `ls`

- **Schema/usage:** optional directory `path` and `limit`.
- **Dispatch/execution:** reads one directory via the filesystem adapter, includes dotfiles, sorts entries, and appends `/` to directories.
- **Output/errors:** one entry per line, bounded by count/truncation with an omitted-entry notice. Missing/non-directory/inaccessible paths throw.
- **Implementation:** `packages/coding-agent/src/core/tools/ls.ts`.
