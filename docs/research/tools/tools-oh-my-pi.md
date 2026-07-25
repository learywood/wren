# oh-my-pi tools

Reference: `can1357/oh-my-pi` at `639bac5`.

## Architecture

oh-my-pi uses `AgentTool<TSchema,TDetails,TRenderer>` objects. A tool supplies an ArkType parameter schema, metadata (name, label, description, strictness, load mode, approval tier, concurrency/interruptibility), async execution, and optional streaming/TUI renderers. A session builds a context-sensitive catalog: “essential” definitions are sent eagerly; “discoverable” tools can be found through the `xd://` resource/search system. Extension factories and subprocess-specific handlers augment built-ins.

The agent loop looks up each call, validates/coerces arguments, requests read/write/exec approval, applies before/after hooks, and executes with an abort signal and update callback. Tools may be sequential, parallel, or exclusive; background work is registered with `AsyncJobManager`. Results are content blocks plus typed `details` and optional `isError`. Thrown exceptions become error results; many domain tools instead return actionable error text. File/shell/search output is bounded, while larger agent/browser/debug outputs can be retained as artifacts or internal resources.

Implementation: `packages/agent/src/types.ts`, `packages/agent/src/agent-loop.ts`, `packages/ai/src/utils/validation.ts`, `packages/coding-agent/src/tools/index.ts`, `essential-tools.ts`, and `builtin-names.ts`.

## Tools

### `read`
Schema: path plus optional offset/limit. Cwd-scoped filesystem execution reads text or images; used for bounded inspection. Returns content blocks, truncation/continuation metadata, and errors for bad ranges, missing/directories, encoding, or I/O. Implementation: `packages/coding-agent/src/tools/read.ts`.

### `write`
Schema: path and complete content. Creates parents and writes through the session writethrough/LSP pipeline; used for create/replace. Returns write and optional diagnostics details; scope, size, formatting, and I/O failures are surfaced. Implementation: `packages/coding-agent/src/tools/write.ts`, `packages/coding-agent/src/lsp/index.ts`.

### `edit`
Schema is selected by configured edit mode: exact replace, patch, hashline-anchored edit, or multi-file apply-patch. The mode executor validates its full operation set, writes through the LSP pipeline, and can batch diagnostics. Returns per-file diffs/snapshots/diagnostics; stale or ambiguous anchors/context, overlap, parse, and I/O failures stop later dependent entries and identify skipped files. Implementation: `packages/coding-agent/src/edit/index.ts` and `edit/modes/`.

### `bash`
Schema: command, timeout, and execution/background controls. Spawns the configured shell, streams merged output, supports abort/timeout, and may register async jobs. Returns bounded output/status or a job handle; spawn failures throw and non-zero/timeout/cancel states retain captured output. Implementation: `packages/coding-agent/src/tools/bash.ts`, `packages/coding-agent/src/async/`.

### `grep`
Schema: regex `pattern`, optional semicolon-delimited path/file/glob/range selector, case sensitivity, gitignore flag, and file-page `skip`. Dispatches native repository search and groups matches with per-file and total caps. Returns bounded file/line matches plus pagination/partial-file notes; invalid regex/selector, native timeout, and search failures error. Implementation: `packages/coding-agent/src/tools/grep.ts`.

### `glob`
Schema: optional path/glob/semicolon-delimited selectors, hidden and gitignore flags, and limit (hard max 200). Runs native path discovery; used to locate files, not contents. Returns bounded sorted paths and limit metadata; malformed selectors, inaccessible roots, timeout, and backend failures error. Implementation: `packages/coding-agent/src/tools/glob.ts`.

### `ast_grep`
Schema: AST `pat`, optional semicolon-delimited path/file/glob/internal-URL selector, and match `skip`. Runs native syntax-aware structural search with language inferred per file and fixed result bounds. Returns ordered source ranges/captures plus parse/limit metadata; invalid patterns/selectors and native/parser failures error. Implementation: `packages/coding-agent/src/tools/ast-grep.ts`.

### `ast_edit`
Schema: non-empty `ops[]` of AST `pat`/replacement `out`, and non-empty path/file/glob/internal-URL `paths[]`. Computes native structural rewrites, previews through the resolve/approval handler, then applies accepted changes. Returns per-file changes, replacement counts, parse errors, and applied state; invalid patterns, file caps, overlaps, parse, stale source, and write failures abort. Implementation: `packages/coding-agent/src/tools/ast-edit.ts`.

### `lsp`
Schema: action (`status`, diagnostics, hover, definition, references, symbols, rename, code actions, etc.), optional file/line/symbol/query/new name/apply/timeout. Routes to cwd-specific language-server clients; read actions query, write actions can apply workspace edits. Returns formatted protocol data and typed success details; missing action fields often return corrective text, while abort/timeout/client failures are bounded and server availability is reported. Implementation: `packages/coding-agent/src/lsp/index.ts`.

### `ask`
Schema: one or more structured questions with choices/free-text controls. Dispatches to the interactive session and blocks for human input. Returns selected/typed answers; unavailable UI, abort, or invalid question structure fails/cancels explicitly. Implementation: `packages/coding-agent/src/tools/ask.ts`.

### `inspect_image`
Schema: image path and optional analysis prompt/detail controls. Loads a local image and invokes the configured vision path. Returns textual inspection plus image metadata; missing/unsupported/oversized images and model failures error. Implementation: `packages/coding-agent/src/tools/inspect-image.ts`.

### `browser`
Schema: action `open|close|run`, named tab, optional URL/app/CDP/viewport/navigation/dialog controls, JavaScript body, timeout, and close-all/kill flags. A stateful controller opens headless, spawned, connected, or cmux tabs; `run` executes code with page/browser/tab helpers. Returns observation/result text, screenshots, URL/viewport, and output metadata; unknown tabs, app/CDP/JavaScript/navigation/timeout/protocol failures error. Implementation: `packages/coding-agent/src/tools/browser.ts` and `tools/browser/`.

### `debug`
Schema: debugger operation plus target/session/breakpoint/evaluation fields. Controls a debugger adapter and persistent debug session. Returns stopped frames, variables, console output, and status; invalid state/IDs, adapter startup, timeout, and command failures error. Implementation: `packages/coding-agent/src/tools/debug.ts`, `packages/coding-agent/src/debug/`.

### `eval`
Schema: code, runtime/language, input/context, timeout, and optional output schema. Executes a bounded evaluator/subprocess, optionally validating structured output. Returns value/stdout/stderr/details; syntax/runtime/timeout/schema failures are reported without hiding output. Implementation: `packages/coding-agent/src/tools/eval.ts`, `packages/coding-agent/src/eval/`.

### `gh`
Schema: GitHub operation with repository/issue/PR/query/pagination and mutation fields. Routes through GitHub API/CLI helpers; read and write operations receive different approval. Returns normalized JSON/text; authentication, validation, API/rate-limit, and command errors are surfaced. Implementation: `packages/coding-agent/src/tools/gh.ts`.

### `web_search`
Schema: query plus result count/search options. Calls the configured web-search provider. Returns ranked title/URL/snippet results with bounded count; provider configuration, network, quota, and malformed response failures error. Implementation: `packages/coding-agent/src/web/search/index.ts`.

### `task`
Schema is dynamic: a flat agent/task spawn or optional batch `context` + `tasks[]`, with model, isolation, output-schema, and async controls. Discovers bundled/user/project agent definitions, preflights policy, then runs structured subagents inline or as bounded background jobs. Returns final structured output or job IDs/progress/details; shape, unknown/disabled agent, recursion, policy, schema, cancellation, and child failures produce corrective results. Implementation: `packages/coding-agent/src/task/index.ts`, `task/types.ts`, `task/structured-subagent.ts`.

### `hub`
Schema: `op` over peer messaging, async jobs, and supervised processes, with recipient/message/job IDs or launch/log/stdin/readiness fields. Routes to `AgentRegistry`, `AsyncJobManager`, or launch broker; supports blocking waits and streamed follow. Returns messages, snapshots, logs, cursors, and process state; unavailable subsystems and operation-specific missing/mutually-exclusive fields return `hubErrorResult`. Implementation: `packages/coding-agent/src/tools/hub/index.ts`, `hub/messaging.ts`, `hub/jobs.ts`, `hub/launch.ts`.

### `todo`
Schema: one operation (`init|start|done|rm|drop|append|view`) with phased lists, task, phase, or items. Applies an atomic transition to session todo state under exclusive concurrency. Returns a rendered state snapshot and completion transitions; invalid/missing/duplicate targets set `isError` and preserve the previous state. Implementation: `packages/coding-agent/src/tools/todo.ts`.

### `learn`
Schema: durable `memory`, optional source `context`, and optional managed-skill `{action:create|update,name,description,body}`. Persists the lesson to the configured Hindsight, Mnemopi, or local backend and optionally writes the skill in the same call. Returns memory/skill status; disabled or uninitialized backends, authored-skill name conflicts, invalid names, and persistence failures error. Implementation: `packages/coding-agent/src/tools/learn.ts`.

### `checkpoint`
Schema: required investigation `goal`. At top level, records an active checkpoint through session state so subsequent investigation can later be discarded while retaining a report. Returns goal/timestamp state; subagents and a second active checkpoint are rejected. Implementation: `packages/coding-agent/src/tools/checkpoint.ts`.

### `rewind`
Schema: required investigation `report`. Completes the active top-level checkpoint and asks the session rewind machinery to restore checkpoint state while retaining the findings. Returns report/rewound metadata; subagent use, no active checkpoint, repeated rewind, and restoration failures error. Implementation: `packages/coding-agent/src/tools/checkpoint.ts`.

### `recall`
Schema: memory query with scope/limit controls. Searches persistent memory and is read-tier. Returns ranked memory fragments; unavailable store and query/read failures are reported. Implementation: `packages/coding-agent/src/tools/memory-recall.ts`.

### `retain`
Schema: non-empty `items[]`, each with memory `content` and optional source `context`. Queues/stores every item in the configured Hindsight or Mnemopi bank. Returns retention count/status; unavailable backend/session and persistence failures error. Implementation: `packages/coding-agent/src/tools/memory-retain.ts`.

### `reflect`
Schema: subject/context and optional memory scope. Synthesizes higher-level conclusions from session/memory material. Returns reflection and provenance; unavailable context/provider and memory/model failures error. Implementation: `packages/coding-agent/src/tools/memory-reflect.ts`.

### `memory_edit`
Schema: `op:update|forget|invalidate`, memory `id`, and optional replacement content/importance/replacement ID. Dispatches only to Mnemopi scoped-memory editing. Returns backend status including not-found/not-editable as text; update without content/importance, uninitialized backend, invalid IDs, and storage failures error. Implementation: `packages/coding-agent/src/tools/memory-edit.ts`.

### `manage_skill`
Schema: `action:create|update|delete`, kebab-case name, and description/body required together for create/update. Writes or deletes an isolated managed `SKILL.md`, then refreshes discovery. Returns action/name/path details; malformed names, authored-skill shadowing, missing fields, and filesystem/refresh failures error. Implementation: `packages/coding-agent/src/tools/manage-skill.ts`, `autolearn/managed-skills.ts`, and `extensibility/skills.ts`.

### `yield`
Schema is generated from the parent’s requested output schema and wraps `{result:{data|error}}`; optional `type:[section]` submits incremental sections and `type:"result"` finalizes. The subprocess registry extracts details and terminates only on terminal yield. Returns submitted/aborted status; empty results and schema mismatches throw retryable errors with hard retry ceilings, after which the child aborts or validation is explicitly overridden. Implementation: `packages/coding-agent/src/tools/yield.ts`.

### `goal`
Schema: internal goal-state operation and goal payload. Used by goal-mode orchestration rather than normal catalog discovery to record/advance success criteria. Returns structured goal state; malformed transitions and unavailable goal context error. Implementation: `packages/coding-agent/src/goals/tools/goal-tool.ts`.

## Catalog boundary

`builtin-names.ts` is the authoritative built-in-name union at this revision. `yield` and `goal` are hidden/internal orchestration tools. Several capabilities (`lsp`, browser/debug services, agents, memory, skills) are configuration-dependent, and non-built-in extension tools may be added at runtime; those are outside this pinned built-in inventory.
