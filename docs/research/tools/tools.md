# Coding-agent tool systems

This directory records how the checked-in reference harnesses expose and execute tools. It is descriptive, not a recommendation or a Wren API proposal. Findings are pinned to the reference revisions below.

| Harness | Revision | Language | Tool-system shape | Catalog |
|---|---:|---|---|---|
| [oh-my-pi](tools-oh-my-pi.md) | `639bac5` | TypeScript | Large, dynamically disclosed `AgentTool` catalog; ArkType schemas; approvals, hooks, streaming, background jobs, and custom rendering | Broad: files, shell, search, LSP/AST, browser/web/GitHub, agents, process supervision, memory, skills, planning |
| [Pi](tools-pi.md) | `5bc1c2c` | TypeScript | Small fixed core of TypeBox `AgentTool`s over pluggable file/operation backends | Seven Unix-like coding primitives |
| [pi_agent_rust](tools-pi-agent-rust.md) | `9fcdb65` | Rust | One object-safe `Tool` trait and registry; explicit JSON Schema and side-effect metadata | Pi-like eight-tool core, adding hash-anchored editing |
| [rust-code](tools-rust-code.md) | `e8245c0` | Rust | `sgr_agent::agent_tool::Tool`; `schemars` argument schemas; eager and deferred registry entries | Broad application-level catalog including git, planning, MCP, agents, and APIs |
| [VTCode](tools-vtcode.md) | `19ace77` | Rust | Link-time distributed registrations grouped into logical `ToolPack`s; policy/capability metadata and native handlers | Canonical public tools plus aliases and hidden implementation dispatchers |

## Cross-harness observations

- **Contracts:** all five publish a name, description, and JSON-compatible argument schema. TypeScript harnesses retain a typed schema object; Rust harnesses either return JSON Schema directly or derive it from argument structs.
- **Dispatch:** Pi and `pi_agent_rust` use direct name-to-object registries. `rust-code` maps names to trait objects and can defer definitions. oh-my-pi builds a session-specific catalog. VTCode assembles distributed registrations and routes aliases to canonical native executors.
- **Scheduling:** mutating Pi calls are queued; `pi_agent_rust` explicitly declares effects; `rust-code` parallelizes read-only calls and serializes mutations. oh-my-pi has per-tool concurrency/approval metadata and async job infrastructure. VTCode combines policy, capability, and handler metadata.
- **Results:** each system returns model-visible text, with some supporting images or structured details. Pi and its Rust port make truncation explicit; larger harnesses additionally persist artifacts, stream progress, or provide domain-specific structured output.
- **Errors:** schema rejection occurs before execution where strict validation is enabled. Runtime failures are either thrown/returned as harness errors or represented as tool results marked erroneous. Several tools deliberately return corrective text for recoverable invocation mistakes.

## Reading convention

Each harness document first describes its architecture, then gives a terse technical entry for every canonical built-in tool in the inspected catalog. “Schema” summarizes model-visible arguments rather than reproducing JSON Schema. “Output/errors” covers result shape, bounding, and notable failure behavior. Repository-relative paths refer to the pinned checkout listed in `docs/references/github.com`.
