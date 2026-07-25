# rust-code tools

Reference: `fortunto2/rust-code` at `e8245c0`.

## Architecture

`Tool` is an async Rust trait with name, description, JSON Schema, `execute(args, AgentContext)`, and flags such as `is_read_only`/`is_system`. Most argument structs derive `schemars::JsonSchema`; `parse_args` converts JSON and returns `ToolError` on malformed input. `ToolOutput` is primarily textual.

`ToolRegistry` stores trait objects in insertion order, resolves names case-insensitively with a conservative Levenshtein fallback, and supports deferred tools: the model initially sees a schema-less stub and must promote the definition before execution. The application registry wires shared cwd/state, MCP/API registries, swarm state, and external-delegate state into tools. The agent loop executes read-only calls concurrently and mutating calls sequentially, feeds each output back as a tool result, and detects repeated calls/stagnant output. Some domain failures are intentionally returned as ordinary explanatory text rather than `ToolError`.

Implementation: `crates/sgr-agent-core/src/agent_tool.rs`, `crates/sgr-agent/src/registry.rs`, `crates/sgr-agent/src/agent_loop.rs`, and `crates/rc-cli/src/agent.rs`.

## Tools

The entries below cover the application registry built in `crates/rc-cli/src/agent.rs`.

### `read_file`
Schema: `path`, optional line range. Resolves against shared cwd and reads a bounded text slice. Returns numbered text; missing/non-file/range/I/O failures become `ToolError`. Implementation: `crates/rc-cli/src/tools/read_file_tool.rs`.

### `write_file`
Schema: `path`, complete `content`. Resolves cwd, creates parents, and replaces the file. Returns confirmation; path/create/write failures are errors. Implementation: `crates/rc-cli/src/tools/write_file_tool.rs`.

### `edit_file`
Schema: `path`, `old_text`, `new_text`. Reads and requires an exact occurrence before replacement. Returns confirmation/diff-oriented text; absent or ambiguous content and I/O fail. Implementation: `crates/rc-cli/src/tools/edit_file_tool.rs`.

### `apply_patch`
Schema: patch text in the harness patch format. Parses and applies multi-file add/update/delete hunks through the patch engine. Returns an application summary; malformed hunks, context mismatch, and filesystem failures return tool errors. Implementation: `crates/rc-cli/src/tools/apply_patch_tool.rs`, `crates/sgr-agent-tools/src/apply_patch.rs`.

### `bash`
Schema: shell `command` and optional timeout/cwd controls. Executes synchronously through the shell helper. Returns stdout/stderr/status after shared output truncation; spawn, timeout, and execution failures are surfaced as errors/text with captured output. Implementation: `crates/rc-cli/src/tools/bash_tool.rs`, `bash.rs`.

### `bash_bg`
Schema: `name`, `command`. Starts a named tmux-backed background command rather than blocking. Returns session instructions/status; duplicate/invalid names and tmux/startup failures are explanatory results. Implementation: `crates/rc-cli/src/tools/bash_tool.rs`, `bash.rs`.

### `search_code`
Schema: query/pattern with optional path, file glob, and result bound. Runs repository text search from cwd and is marked read-only. Returns bounded matches; no matches are success, while invalid search and subprocess failures error. Implementation: `crates/rc-cli/src/tools/search_tool.rs`.

### `git_status`
Schema: no substantive arguments. Executes repository status and is read-only. Returns porcelain/status text; missing repository or git failure is reported. Implementation: `crates/rc-cli/src/tools/git_tool.rs`.

### `git_diff`
Schema: optional staged/path controls. Executes the corresponding diff and is read-only. Returns bounded diff text; git errors are reported. Implementation: `crates/rc-cli/src/tools/git_tool.rs`.

### `git_add`
Schema: paths to stage. Runs `git add`; returns confirmation or command error. It is mutating and therefore serialized. Implementation: `crates/rc-cli/src/tools/git_tool.rs`.

### `git_commit`
Schema: commit `message`. Runs commit with configured cwd/state. Returns command output; empty/failed commits and git failures are reported. Implementation: `crates/rc-cli/src/tools/git_tool.rs`.

### `open_editor`
Schema: file path and optional line. Launches the configured editor. Returns launch confirmation; missing editor/path and process-start failures are reported. Implementation: `crates/rc-cli/src/tools/editor_tool.rs`.

### `finish`
Schema: final response/message. Marks the agent task complete and returns final textual output through context state. Bad argument shape is `ToolError`. Implementation: `crates/rc-cli/src/tools/finish_tool.rs`.

### `ask_user`
Schema: question text. Emits a user-input request via the application interaction path. Returns the answer or an unavailable/cancelled explanation; parsing errors fail. Implementation: `crates/rc-cli/src/tools/finish_tool.rs`.

### `update_plan`
Schema: ordered plan steps with statuses. Replaces/updates plan state in `AgentContext`; intended for progress tracking. Returns a plan summary and rejects malformed statuses/schema. Implementation: `crates/sgr-agent-tools/src/plan.rs`.

### `mcp_call`
Schema: MCP server, tool name, and arbitrary arguments. Looks up the configured `McpManager` and dispatches remotely. Returns serialized MCP content; absent manager/server/tool and protocol failures are reported. Implementation: `crates/rc-cli/src/tools/mcp_tool.rs`, `mcp.rs`.

### `memory`
Schema: operation plus memory key/content/query fields. Reads or updates persistent project/user memory according to operation. Returns matching/stored text; unknown operations and storage failures are reported. Implementation: `crates/rc-cli/src/tools/memory_tool.rs`.

### `project_map`
Schema: optional `path`. Calls `solograph::generate_repomap` and is read-only. Returns the generated structural map; argument parsing is the principal typed failure boundary. Implementation: `crates/rc-cli/src/tools/project_tools.rs`.

### `dependencies`
Schema: optional manifest `path`. Detects Cargo, npm, or Python manifests and parses dependencies with `solograph`; read-only. Returns a normalized list or “none found”; parse absence is ordinary text. Implementation: `crates/rc-cli/src/tools/project_tools.rs`.

### `task`
Schema: `operation` (`create|list|update|done`) plus optional title, numeric id, status, priority, and notes. Mutates/reads `.tasks` state according to operation. Returns task summaries; missing IDs, unknown operations, and absent tasks are corrective text rather than hard errors. Implementation: `crates/rc-cli/src/tools/task_tool.rs`.

### `spawn_agent`
Schema: role, task, optional max steps. Builds a subagent and registers it in `SwarmManager`. Returns agent ID; no provider and spawn failures are ordinary text. Implementation: `crates/rc-cli/src/tools/swarm_tools.rs`.

### `wait_agents`
Schema: optional agent ID list and timeout seconds. Waits for selected/all swarm children and gathers results. Returns per-agent text, including timeout states; an empty swarm succeeds immediately. Implementation: `crates/rc-cli/src/tools/swarm_tools.rs`.

### `agent_status`
Schema: optional agent ID. Read-only lookup in `SwarmManager`. Returns one/all statuses; unknown IDs are textual results. Implementation: `crates/rc-cli/src/tools/swarm_tools.rs`.

### `cancel_agent`
Schema: required agent ID or `all`. Cancels one/all swarm jobs. Returns confirmation or textual cancellation failure. Implementation: `crates/rc-cli/src/tools/swarm_tools.rs`.

### `api`
Schema: optional API name plus operation/path/method/parameters/body fields. Resolves a registered OpenAPI service and invokes it. Returns normalized response text after truncation; unknown API/operation, invalid arguments, transport, and response failures are reported. Implementation: `crates/rc-cli/src/tools/api_tool.rs`.

### `delegate_task`
Schema: CLI agent, optional free-text task or task-file path, optional cwd. Starts Claude/Gemini/Codex/OpenCode/rust-code in a tmux-backed `DelegateManager`. Returns delegate ID; unknown agent and spawn failures are text. Implementation: `crates/rc-cli/src/tools/delegate_tools.rs`, `delegate.rs`.

### `delegate_status`
Schema: optional delegate ID. Read-only status of one/all external delegates. Returns state and elapsed time; unknown/empty results are ordinary text. Implementation: `crates/rc-cli/src/tools/delegate_tools.rs`.

### `delegate_result`
Schema: delegate ID. Read-only retrieval of completed external output. Returns a shared-truncated transcript; missing/running/read failures are explanatory text. Implementation: `crates/rc-cli/src/tools/delegate_tools.rs`.
