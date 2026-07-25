# VTCode tools

Reference: `vinhnx/VTCode` at `19ace77`.

## Architecture

VTCode separates declaration, policy, and execution. `ToolRegistration` binds a canonical name to capability level, description, JSON Schema, aliases, model visibility, permission policy, behavior metadata, and a native CGP handler/factory. Built-ins self-register into a `linkme` distributed slice; startup sorts registrations deterministically and loads logical `ToolPack`s (`hitl`, planning, multi-agent, search, web, shell, internal PTY, editing). Runtime session setup adds context-dependent skill tools.

Schemas shared with other surfaces live in `vtcode-utility-tool-specs`; native tool types can supply their own. Aliases resolve to canonical registrations, while action-bearing tools consolidate formerly separate names. The router applies capability and `ToolPolicy` checks before invoking a `ToolRegistry::*_executor`; sandbox policy additionally governs shell/filesystem/network operations. Executors return normalized tool responses and structured errors from the registry error layer. Model visibility is independent of registration: hidden helpers remain callable by harness adapters and public tools but are not advertised to the model.

Implementation: `crates/codegen/vtcode-core/src/tools/registry/{distributed,registration,router,error,builtins,pack_impls}.rs`, `crates/codegen/vtcode-core/src/tools/handlers/`, `crates/codegen/vtcode-config/src/constants/tools.rs`, and `crates/utils/vtcode-utility-tool-specs/`.

## Model-visible canonical tools

### `request_user_input`
Schema: structured prompt/question definitions and answer options. Native CGP dispatch asks the interactive user. Returns structured answers; invalid forms, unavailable UI, and cancellation error explicitly. Implementation: `tools/request_user_input.rs`; registration in `registry/builtins.rs`.

### `memory`
Schema: action plus `/memories` path/content. `memory_executor` lists, reads, or updates persistent memory; writes are restricted to `preferences.md`, `repository-facts.md`, and `notes/**`. Returns listings/content/update status; traversal, disallowed targets, malformed actions, and I/O fail. Implementation: `tools/native_memory.rs`; registration in `registry/builtins.rs`.

### `cron`
Schema: action `create|list|delete` with prompt/schedule/id fields. `cron_executor` manages session-scoped scheduled prompts. Returns schedule records/status; invalid cadence/action/ID and scheduler failures error. Aliases include `cron_create`, `cron_list`, and `cron_delete`. Implementation: `tools/registry/executors.rs`; schema in `vtcode-utility-tool-specs`.

### `start_planning`
Schema supplied by `StartPlanningTool` for entering the planning workflow. Native handler transitions `PlanningWorkflowState` and constrains subsequent capabilities. Returns plan-mode state; invalid/repeated transitions and state persistence failures are reported. Implementation: `tools/handlers/planning_workflow/`; registration in `registry/builtins.rs`.

### `task_tracker`
Schema: action `create|update|list|add` plus checklist/task/status fields. Native handler mirrors tracker state between `.vtcode/tasks/current_task.md` and active plan sidecars. Returns current checklist; invalid IDs/statuses or persistence conflicts fail. Aliases: `plan_manager`, `track_tasks`, `checklist`. Implementation: `tools/handlers/task_tracker.rs` and `planning_task_tracker.rs`; registration in `registry/builtins.rs`.

### `agent`
Schema: action `spawn|spawn_subprocess|send_input|resume|wait|close` with child/task/input/timeout fields. `agent_executor` drives delegated agents and managed subprocesses. Returns child IDs, progress, or results; recursion/capacity, unknown child, timeout, cancellation, and child failures are normalized. Former lifecycle names are aliases. Implementation: `tools/registry/executors/subagents.rs`; schema `agent_parameters` in `vtcode-utility-tool-specs`.

### `code_search`
Schema: required literal query (supports `|` alternatives), optional path, file types, result types, and maximum. `code_search_executor` combines definition/usage/text/path search under `CodeSearch` capability. Returns bounded typed matches; invalid filters/path and indexing/search failures error. Implementation: `tools/code_search/` and `tools/registry/executors.rs`; shared schema in `vtcode-utility-tool-specs`.

### `web_fetch`
Schema: URL, optional analysis prompt, `max_bytes` (default 500,000), timeout (default 30s). Native `WebFetchTool` performs policy-checked network retrieval and content analysis. Returns bounded extracted/analysed content; URL policy, size, timeout, HTTP, and decode failures error. Permission defaults to Prompt; aliases `fetch_url`, `web`. Implementation: `tools/web_fetch.rs`.

### `web_search`
Schema: query and optional `max_results` (default 8, max 20). Native DuckDuckGo search returns ranked results. Invalid/empty query, network/rate-limit, parse, and timeout failures error; permission defaults to Prompt. Aliases `search_web`, `websearch`. Implementation: `tools/web_search.rs`.

### `mcp`
Schema: action `search_tools|get_tool_details|list_servers|connect|disconnect` with server/query/tool fields. `mcp_executor` discovers schemas and manages server lifecycle; connect/disconnect receive action-qualified policy checks. Returns catalogs/details/server state; unknown/disconnected servers, active-call conflicts, transport, and protocol failures error. Legacy MCP operation names are aliases. Implementation: `tools/mcp.rs`, `tools/registry/mcp_facade.rs`, and `registry/executors.rs`; shared `mcp_parameters`.

### `exec_command`
Schema: command plus cwd, timeout/yield/output/session controls. `exec_command_executor` runs through sandbox and permission checks, returning output, exit status, and a reusable session ID if still running. Spawn/policy/timeout/cancellation failures retain available output. Implementation: `tools/registry/executors.rs` and shell handlers; shared `exec_command_parameters`.

### `write_stdin`
Schema: active execution session ID, characters/input, and polling controls. `write_stdin_executor` writes to the command session and returns fresh output/status. Unknown/closed sessions, invalid input, and I/O errors fail. Implementation: `tools/registry/executors.rs` and shell handlers; shared `write_stdin_parameters`.

### `apply_patch`
Schema: VTCode patch text (`*** Begin Patch`, file operation headers, hunks). `apply_patch_executor` parses, permission-checks, and atomically applies multi-file changes. Returns changed-file/hunk summary; standard-diff syntax, malformed/context-stale hunks, sandbox denial, and I/O errors abort. Permission defaults to Prompt. Implementation: `tools/apply_patch.rs`; shared `apply_patch_parameters`.

## Registered hidden tools

These have full registrations and executors but `llm_visibility(false)` at this revision.

### `defuddle_fetch`
Schema: HTTP(S) URL and optional `max_bytes` capped at 262,144. Native one-shot defuddle service returns extracted Markdown; rejects local URLs, repeated use, HTTP/rate-limit, size, and parse failures. Aliases `defuddle`, `extract_markdown`. Implementation: `tools/defuddle.rs`.

### `exec_pty_cmd`
Uses the `exec_command` schema but dispatches through `run_pty_cmd_executor` with a controlling PTY. Returns output/status/session ID; PTY allocation, sandbox, timeout, and process errors fail. Implementation: registry handlers and PTY manager.

### `read_file`
Schema supplied by its handler: path plus chunk/range or indentation-aware selection. Line-based reads return unprefixed bounded text in a structured response containing the path, success state, and optional `has_more`; the separate byte-range mode prefixes line numbers. Path policy, range, binary/size, and I/O errors fail. Implementation: `tools/handlers/read_file.rs`, `tools/file_ops/mod.rs`, and `tools/registry/executors.rs`.

### `list_files`
Schema: path, pagination, and listing filters. `list_files_executor` returns bounded files/directories and continuation state; invalid/inaccessible directories and pagination values error. Implementation: shared `list_files_parameters`, `tools/handlers/list_dir_handler.rs`, and `tools/registry/executors.rs`.

### `write_file`
Schema: path and complete content. `write_file_executor` performs sandboxed create/overwrite. Returns write status; policy, path, size, and I/O failures error. Implementation: `tools/registry/executors.rs` and filesystem handlers.

### `edit_file`
Schema: path and surgical search/replacement fields. `edit_file_executor` verifies current content and writes the result. Returns edit summary; absent/ambiguous/stale targets and policy/I/O failures abort. Implementation: `tools/registry/executors.rs` and filesystem handlers.

### `run_pty_cmd`
Schema: PTY command/session options. Internal one-shot PTY executor returns output/status; allocation/process/timeout errors fail. Implementation: PTY handlers.

### `send_pty_input`
Schema: PTY session ID and input/keys. Internal executor writes to the terminal and returns fresh output; unknown session and write failures error. Implementation: PTY handlers.

### `read_pty_session`
Schema: PTY session ID and output range/poll options. Returns buffered terminal output and cursor/status; unknown session and read failures error. Implementation: PTY handlers.

### `create_pty_session`
Schema: command, cwd, environment, and terminal sizing options. Creates a persistent interactive terminal and returns its ID; sandbox/allocation/spawn failures error. Implementation: PTY handlers.

### `list_pty_sessions`
Schema has no required fields. Returns active PTY IDs and state; manager failures are normalized. Implementation: PTY handlers.

### `close_pty_session`
Schema: PTY session ID. Closes/terminates the session and returns status; unknown IDs and termination failures error. Implementation: PTY handlers.

### `get_errors`
Schema: optional scope/filter over the latest compilation/lint run. `get_errors_executor` returns normalized diagnostics; absent run data is an empty/diagnostic response and malformed filters fail. Implementation: diagnostic handlers.

## Runtime skill tools

These are canonical tools but are created in session setup rather than the distributed built-in slice.

### `list_skills`
Schema: optional discovery/filter fields. Lists local and dormant system skills with metadata; scan/config failures error. Implementation: session setup and skill handlers; constants in `vtcode-config/src/constants/tools.rs`.

### `load_skill`
Schema: skill name. Activates its instructions and tools in the live registry. Returns loaded instructions/tool set; unknown, invalid, or conflicting skills and registration failures error. Implementation: session setup and skill handlers.

### `load_skill_resource`
Schema: skill name and resource path. Reads a skill script/template/document under its package boundary. Returns bounded resource content; traversal, missing resource, and I/O errors fail. Implementation: session setup and skill handlers.

## Catalog boundary

Aliases are not duplicate implementations: the router resolves them to the canonical registration, often injecting/inferring an action. Constants such as `search_dispatch_internal`, `command_session_internal`, and `file_operation_internal` name lower-level dispatchers rather than additional distributed built-ins. The inventory may also contain MCP- or skill-provided tools at runtime; those are external to this pinned native catalog.
