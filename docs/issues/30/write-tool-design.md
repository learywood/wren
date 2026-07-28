# Baseline write tool design

> **Issue:** [#30 — Ship a baseline write tool extension](https://github.com/learywood/wren/issues/30)

## Scope and evidence boundary

This document recommends the smallest Windows-only `write` extension that satisfies issue #30. It is research and design, not an implementation plan for edit syntax, model orchestration, or filesystem containment.

The recommendation is bounded as follows:

- one bundled native extension using Wren's existing API revision 2 contract;
- required `path` and complete string `content` arguments;
- relative paths resolved from `ToolContext::working_directory()`;
- file creation, replacement, and missing-parent creation;
- trusted in-process operation with the Wren process's privileges, not a security sandbox;
- installation through `cargo install-wren` beside the existing read extension;
- deterministic unit and installed-release functional claims only; and
- agent effectiveness deferred to the evaluation work under #29.

Hashlines, exact replacements, patches, append modes, conditional-create modes, arbitrary commands, multi-file transactions, and a general installer system are outside this issue.

The Wren tree was inspected at `8028f272c469f0b0595430eb6a39a8550935fdf8`. The current contract is synchronous, passes a JSON object and working directory to a tool, and returns text plus optional JSON details or a kind/message error. It has no cancellation or scheduling primitive (`crates/wren-extension/src/lib.rs`). The loader, CLI boundary, and installed-extension layout are defined in `src/extension.rs`, `src/main.rs`, and `docs/architecture/extension-contract.md`.

In the findings below:

- **Observed** means the cited source or test establishes the behavior.
- **Inferred** means the behavior follows from the cited primitive but is not established by a focused test in that repository.
- **Recommended** means a Wren product decision in this document.

Reference checkouts were reused from the location prescribed by `docs/references/github.com`, inspected without modification, and pinned to the revisions below. The other catalog entries, `agentclientprotocol/agent-client-protocol` and `0xPlaygrounds/rig`, are a protocol and a general framework rather than directly relevant bundled coding-agent file-mutation implementations, so no additional implementation was included.

## Inspected references and exact revisions

| Repository | Revision inspected | Relevant implementation and tests |
|---|---|---|
| `earendil-works/pi` | `5bc1c2c0a6f07e00e8c240304182f213ab8d311f` | `packages/coding-agent/src/core/tools/write.ts`, `file-mutation-queue.ts`, `path-utils.ts`, `edit.ts`; `packages/coding-agent/test/file-mutation-queue.test.ts` |
| `can1357/oh-my-pi` | `639bac596d94b5993349f3f6696176cb2bf9b5d3` | `packages/coding-agent/src/tools/write.ts`, `prompts/tools/write.md`, `lsp/index.ts`, `tools/path-utils.ts`; `packages/coding-agent/test/tools.test.ts`, `write-acp-fs.test.ts` |
| `Dicklesworthstone/pi_agent_rust` | `9fcdb655cfb7ef5019af2ca4353d3aa019040329` | `src/tools.rs`, `src/extensions.rs`, `src/agent.rs`; co-located tests and `tests/tools_conformance.rs` |
| `fortunto2/rust-code` | `e8245c0bf2fc81d9feb060314e087231e7694d14` | `crates/rc-cli/src/tools/write_file_tool.rs`, `edit_file_tool.rs`, `rc_state.rs`; `crates/sgr-agent/src/app_tools/fs.rs`, `agent_loop.rs`; `crates/sgr-agent-core/src/agent_tool.rs` |
| `vinhnx/VTCode` | `19ace7724a53f655e737a224ab0ecc8386a34c78` | `crates/codegen/vtcode-core/src/tools/{types.rs,file_ops/write.rs,file_ops/write/fs_ops.rs,file_ops/path_policy.rs,edited_file_monitor.rs,registry/builtins.rs}`; `crates/codegen/vtcode-core/tests/file_conflict_integration.rs` |
| `openai/codex` | `4c43465133428898aa84f0bfc02c306ed65fb66a` | `codex-rs/core/src/tools/handlers/{apply_patch.rs,apply_patch_spec.rs,apply_patch.lark}`, `codex-rs/apply-patch/src/{lib.rs,invocation.rs}`, `codex-rs/core/src/tools/runtimes/apply_patch.rs`, `codex-rs/exec-server/src/local_file_system.rs`; co-located tests |

## Concise comparison matrix

| Harness | Model-facing mutation | Create/replace behavior | Path and parent behavior | Write strategy and coordination | Bounds and result |
|---|---|---|---|---|---|
| Pi | `write(path, content)`; separate exact multi-`edit` | Whole-file create or overwrite | Relative to cwd or absolute; recursively creates parents | Direct UTF-8 write; per-canonical-file queue; abort checks around awaited operations | No write limit; text reports JavaScript string length as “bytes”; no details |
| oh-my-pi | Strict, exclusive `write(path, content)` plus several edit modes | Whole-file route, but hashline stripping or configured formatting can transform content | Relative to cwd or absolute; nested-parent test establishes creation | Ordinary writes go directly through Bun/LSP; exclusive tool scheduling; abort-aware | No ordinary-write limit found; rich diagnostics/details; reported “bytes” are JavaScript string length |
| `pi_agent_rust` | `write(path, content)`; exact `edit` and hashline edit separate | Whole-file create or overwrite | Cwd-contained after canonicalization; creates parents; blocks symlink escapes | Same-directory temporary file, file sync, persist; write effects are scheduling barriers | 100 MiB; text reports UTF-16 units as “bytes”; no details |
| `rust-code` | `write_file(path, content)`; separate deprecated edit and patch | Whole-file create or overwrite | Relative to mutable cwd or absolute; creates parents; no containment | Direct Tokio write; mutating calls run sequentially | No limit; `Created`/`Wrote` plus line count; text-only output |
| VTCode | `write_file` is model-hidden at this revision | Internal helper defaults to fail-if-exists and also supports overwrite/append/skip | Workspace-contained canonical path; creates parents | Direct open/truncate/write/flush; per-file mutation lease and stale-read conflict check | 64,000 UTF-8 bytes; structured JSON with mode, bytes, existence, and diff preview |
| Codex | No whole-file write tool; model sees freeform `apply_patch` | Add/update/delete/move hunks | Relative to cwd or absolute; add retries after recursive parent creation | Direct filesystem writes, sequential hunks, sandbox/approval orchestration; partial committed delta is tracked | No explicit patch limit found in inspected implementation; A/M/D success summary |

These are descriptive comparisons, not parity requirements. In particular, VTCode's modes and Codex's patch transaction reporting solve different product problems from issue #30.

## Reference findings

### Pi

**Observed.** Pi's model-visible definition is named `write`, requires string `path` and `content`, and says that it creates or overwrites a file and automatically creates parent directories. Relative paths are resolved from the session cwd, parents are created recursively, and Node's UTF-8 `writeFile` performs the final create/truncate/write (`packages/coding-agent/src/core/tools/write.ts`, `packages/coding-agent/src/core/tools/path-utils.ts`). There is no write-specific input bound or temporary-file replacement in that path.

**Observed.** Pi serializes edit and write operations that resolve to the same existing real path while allowing different paths to proceed concurrently. Its tests establish same-file ordering, shared edit/write ordering, symlink-alias queue identity, and that cancellation does not release the queue while an underlying write is still in flight (`packages/coding-agent/src/core/tools/file-mutation-queue.ts`, `packages/coding-agent/test/file-mutation-queue.test.ts`). Abort checks occur before and after directory creation and writing; an abort observed after the write can therefore return an error after bytes were committed (`packages/coding-agent/src/core/tools/write.ts`).

**Observed.** Success is `Successfully wrote <content.length> bytes to <path>` with no details. `content.length` is JavaScript UTF-16 code-unit length, not the number of UTF-8 bytes for all strings (`packages/coding-agent/src/core/tools/write.ts`). Runtime filesystem errors are thrown rather than mapped to a stable write-specific category. The plain-file tool has no approval or cwd-containment layer; absolute paths are part of its advertised schema.

**Inferred.** Because the final primitive is a normal direct open/write without a no-follow flag, final file symlinks and parent reparse points receive normal host-filesystem resolution. A write failure after truncation can leave partial content. The Pi tests cited above establish queue behavior, not Windows replacement, lock, permission, durability, BOM, or reparse-point behavior.

Pi's `edit` instead reads the original file, handles BOM and line endings for exact replacements, computes all changes in memory, and writes once under the same queue (`packages/coding-agent/src/core/tools/edit.ts`). That is useful contrast but outside Wren #30.

### oh-my-pi

**Observed.** oh-my-pi exposes a strict, essential, exclusive `write` with required string `path` and `content`; it assigns write approval and describes whole-file create/overwrite (`packages/coding-agent/src/tools/write.ts`, `packages/coding-agent/src/prompts/tools/write.md`). Its path helper accepts absolute paths and resolves relative paths from cwd (`packages/coding-agent/src/tools/path-utils.ts`). The primary tool test establishes ordinary writing and recursive nested-parent creation (`packages/coding-agent/test/tools.test.ts`).

**Observed.** Ordinary filesystem writes route through an ACP bridge or an LSP writethrough. The no-LSP path uses `Bun.write`; the LSP path can format before the final direct write and can return diagnostics (`packages/coding-agent/src/lsp/index.ts`, `packages/coding-agent/src/tools/write.ts`). Hashline mode can also strip display prefixes from submitted content. Consequently its ordinary `write` is not always an exact byte-preserving primitive. The archive-entry branch alone uses a same-directory temporary file and rename; that branch explicitly protects a whole-archive rewrite and is not the ordinary file algorithm (`packages/coding-agent/src/tools/write.ts`).

**Observed.** No ordinary-write content bound appears in `packages/coding-agent/src/tools/write.ts`. Success and progress again label JavaScript string length as bytes; details can include resolved path, diagnostics, and executable-bit state. Execution accepts an abort signal, tool metadata makes writes exclusive, and ordinary write failures surface as thrown `ToolError`/I/O text rather than a small stable filesystem taxonomy (`packages/coding-agent/src/tools/write.ts`). Plain filesystem writes require write approval; absolute paths are permitted rather than cwd-contained.

**Inferred.** The ordinary direct path receives normal host symlink, junction, lock, ACL, and partial-write behavior. The cited tests establish file and parent creation, ACP routing, formatting-related behavior, and results, but not Windows final-reparse or no-share-lock semantics.

The archive, SQLite, internal-URL, hashline, formatter, executable-bit, approval, and device behavior is deliberately not carried into Wren's baseline.

### `pi_agent_rust`

**Observed.** The Rust port exposes `write` with required string `path` and `content`, Pi's create/overwrite/parent description, and a 100 MiB UTF-8 byte bound (`src/tools.rs`). It resolves relative or absolute input, normalizes dot segments, then enforces cwd containment with a helper that canonicalizes the longest existing ancestor. This blocks parent traversal and symlink escapes while permitting symlinks whose targets remain in cwd (`src/tools.rs`, `src/extensions.rs`; tests in `tests/tools_conformance.rs`).

**Observed.** Existing non-regular destinations are rejected and parents are created. The final algorithm writes to a same-directory `NamedTempFile`, calls `sync_all`, best-effort copies existing permissions, then persists it over the destination. Parent-directory sync is implemented only on Unix and is a no-op on Windows (`src/tools.rs`). Tests establish new files, replacement, nested parents, empty content, Unicode, cwd rejection, and Unix symlink handling (`src/tools.rs`, `tests/tools_conformance.rs`). They do not establish locked-file or replacement behavior on Windows.

**Observed.** The tool reports JavaScript-compatible UTF-16 code-unit count as “bytes” and returns no details. Its tool trait has no cancellation parameter. Write effects are barriers in agent batch planning, so mutations are scheduled sequentially (`src/tools.rs`, `src/agent.rs`). Validation and execution failures use the repository's validation/tool error wrappers with corrective strings, not a stable per-filesystem-kind contract. Cwd enforcement is a host-containment policy, but no approval prompt or OS sandbox is part of this write implementation.

This is the strongest reference for atomic replacement, but its containment, 100 MiB limit, permission-copying, and cross-platform durability machinery are not demonstrated needs for Wren #30.

### `rust-code`

**Observed.** The application catalog's tool is `write_file`, with schemars-derived required string `path` and `content` and description `Create or overwrite a file with new content.` It resolves relative input from shared cwd, accepts absolute paths, recursively creates parents, and calls Tokio's direct `fs::write` (`crates/rc-cli/src/tools/write_file_tool.rs`, `crates/rc-cli/src/rc_state.rs`, `crates/sgr-agent/src/app_tools/fs.rs`). There is no containment, input-size bound, temporary file, sync, structured details, or cancellation parameter in this route.

**Observed.** Success distinguishes `Created` from `Wrote` using a pre-write existence check and reports Rust `str::lines()` count. Mutating tools are executed sequentially while read-only calls may run in parallel (`crates/rc-cli/src/tools/write_file_tool.rs`, `crates/sgr-agent-core/src/agent_tool.rs`, `crates/sgr-agent/src/agent_loop.rs`). The co-located filesystem tests establish direct round trips and edit behavior, but use Unix-style temporary paths and do not establish Windows lock or reparse behavior (`crates/sgr-agent/src/app_tools/fs.rs`).

**Inferred.** Direct Tokio writing follows ordinary symlink/junction resolution and can leave a truncated or partial file after a late failure. I/O errors are collapsed to `ToolError::Execution` by the wrapper (`crates/rc-cli/src/tools/write_file_tool.rs`). There is no explicit approval, sandbox, cancellation, or Windows sharing policy in this write route, and the cited tests do not cover those behaviors.

### VTCode

**Observed.** `write_file` is registered with editing capability and an internal-helper description, but is explicitly hidden from the model at this revision and has no schema attached at that registration (`crates/codegen/vtcode-core/src/tools/registry/builtins.rs`). The internal deserializer requires `path` and `content`, accepts several aliases, and also accepts `overwrite`, `encoding`, and `mode`; mode defaults to `fail_if_exists` (`crates/codegen/vtcode-core/src/tools/types.rs`).

**Observed.** The helper limits content to 64,000 UTF-8 bytes, canonicalizes allowed/missing paths under its workspace policy, checks ignore rules, creates parents, and supports overwrite, append, skip-if-exists, and fail-if-exists. Overwrite uses direct create/write/truncate, `write_all`, and `flush`, not temporary replacement (`crates/codegen/vtcode-core/src/tools/file_ops/path_policy.rs`, `crates/codegen/vtcode-core/src/tools/file_ops/write.rs`). It acquires a per-file mutation lease and checks for stale-read conflicts (`crates/codegen/vtcode-core/src/tools/edited_file_monitor.rs`).

**Observed.** Success is structured JSON containing a message, workspace-relative path, effective mode, bytes written, prior existence, and a bounded diff preview (`crates/codegen/vtcode-core/src/tools/file_ops/write.rs`). Failures are corrective `anyhow` messages normalized by the registry rather than a small write-specific filesystem taxonomy. Integration tests establish that stale external changes return a conflict without overwriting disk state, but do not establish Windows lock/reparse behavior (`crates/codegen/vtcode-core/tests/file_conflict_integration.rs`). The write path has no explicit cancellation token; its canonical workspace policy and registry capability/policy layers are host restrictions absent from Wren's trusted baseline.

**Inferred.** Canonicalize-allow-missing resolves existing symlink or junction ancestors before the workspace check, while the final direct open supplies native provider, ACL, sharing, and partial-write behavior (`crates/codegen/vtcode-core/src/tools/file_ops/path_policy.rs`, `crates/codegen/vtcode-core/src/tools/file_ops/write.rs`).

The shell fallback suggested for oversized content, mutation modes, workspace containment, conflict snapshots, and diff previews would all expand Wren beyond issue #30.

### Codex

**Observed.** Codex has no model-visible whole-file write tool at this revision. Its relevant mutation primitive is a freeform `apply_patch` custom tool, described as a file editor whose patch must not be wrapped in JSON. Its grammar supports add, update, delete, and move hunks (`codex-rs/core/src/tools/handlers/apply_patch_spec.rs`, `codex-rs/core/src/tools/handlers/apply_patch.lark`).

**Observed.** Relative patch paths resolve from the effective cwd and absolute paths are accepted. Add/move writes retry after recursively creating a missing parent. Updates read UTF-8 text, match patch context, construct the new content in memory, and write directly through the selected executor filesystem (`codex-rs/apply-patch/src/invocation.rs`, `codex-rs/apply-patch/src/lib.rs`). The direct local executor delegates to Tokio's direct `fs::write` (`codex-rs/exec-server/src/local_file_system.rs`). Add/update content uses LF and update construction ensures a final LF, so this is not an arbitrary exact-byte writer.

**Observed.** Hunks are applied sequentially rather than as a filesystem transaction. The implementation records the definitely committed prefix and marks the delta inexact when a failed write could have truncated or partially modified a destination. Parse, context-match, directory, and I/O failures produce corrective text; success lists A/M/D paths (`codex-rs/apply-patch/src/lib.rs`). Tests establish creation, relative and absolute paths, updates, deletion, and write-failure delta behavior, but not Windows locks or write-through reparse behavior (`codex-rs/apply-patch/src/lib.rs`, `codex-rs/apply-patch/src/invocation.rs`). The runtime layers sandbox and approval policy around application and can retry sandbox denial (`codex-rs/core/src/tools/runtimes/apply_patch.rs`, `codex-rs/core/src/tools/handlers/apply_patch.rs`). There is no patch-level cancellation argument or explicit patch-size limit in the inspected implementation.

**Inferred.** Cancellation of the surrounding async operation can interrupt a multi-hunk patch; the committed delta is the available record of mutations observed before such interruption.

Codex demonstrates useful honesty about partial mutation, but its patch grammar, multi-file delta, sandbox, and approval orchestration are not baseline-write requirements.

## Windows-specific findings

Wren must design for Windows rather than infer Unix rename, sharing, or permission behavior.

1. **Direct create/truncate has clear sharing behavior.** Win32 `CREATE_ALWAYS` truncates an existing writable file and creates a missing file. An incompatible existing handle causes `ERROR_SHARING_VIOLATION`; a handle opened with no sharing blocks later read, write, or delete access. Rust's default Windows `OpenOptions` share mode is `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`, so Wren's own open is not an exclusive lock. These behaviors are documented by Microsoft's [CreateFileW](https://learn.microsoft.com/windows/win32/api/fileapi/nf-fileapi-createfilew) and Rust's [`OpenOptionsExt`](https://doc.rust-lang.org/std/os/windows/fs/trait.OpenOptionsExt.html). Wren's existing installed functional test already constructs a deterministic no-share lock and maps raw errors 5, 32, and 33 to `permission_denied` for read (`tests/functional.rs`, `extensions/read/src/lib.rs`).

2. **Normal opens process reparse points.** `FILE_FLAG_OPEN_REPARSE_POINT` is the opt-out from normal reparse processing; ordinary Rust opens do not set it. Following a final link is therefore the natural direct-write behavior, but it makes “replace this path” ambiguous and can redirect writes. **Recommended:** inspect the final destination with `symlink_metadata` and reject any Windows `FILE_ATTRIBUTE_REPARSE_POINT` before opening. This rejects final symlinks, junctions, mount points, cloud placeholders, and unknown providers uniformly. Parent directory reparse points remain allowed and are followed by normal resolution. This is an operating rule, not a containment claim. The attribute is defined in Microsoft's [File Attribute Constants](https://learn.microsoft.com/windows/win32/fileio/file-attribute-constants).

3. **Temporary replacement is not free robustness.** `ReplaceFileW` requires delete-related access, opens the replacement with no sharing, attempts ACL/attribute merging, and documents exceptional failure states in which names or inherited metadata may already have changed. Rust `rename` maps to Windows rename facilities and has filesystem-dependent replacement behavior. See Microsoft's [ReplaceFileW](https://learn.microsoft.com/windows/win32/api/winbase/nf-winbase-replacefilew) and Rust's [`fs::rename`](https://doc.rust-lang.org/std/fs/fn.rename.html). A correct atomic design would also need to decide file identity, hard-link behavior, ACLs, attributes, temporary cleanup, sharing, and all replacement error states. No issue #30 need demonstrates that complexity.

4. **Permissions are ACLs and attributes, not Unix mode bits.** A read-only attribute or denied write/truncate access fails the open. A temp-file rename may require delete access even when a direct content write would be allowed. The recommended direct open preserves an existing file's identity, ACL, attributes, and hard links because it changes that file's data rather than replacing its directory entry. A hard link is not a reparse point; all links to the file observe the new content.

5. **No durability claim follows from success.** `write_all` establishes that Rust submitted all bytes without an error. Without `sync_all` and a defined volume/filesystem contract it does not establish survival across power loss. Process termination or a late I/O failure after truncation can leave zero or partial content. Wren should say so rather than call the direct write atomic or durable.

6. **Windows path namespaces remain host behavior.** Absolute paths are allowed and no cwd containment is promised. Parent junctions can therefore route parent creation and writing outside the lexical path. Alternate data streams, reserved DOS names, network paths, and provider-specific files are not specially emulated or treated as sandbox escapes; they receive normal Windows errors and semantics. Final destinations carrying the reparse attribute are the one explicit exclusion.

## Recommended Wren contract

### Name and description

- **Extension ID/name:** `write`
- **Tool name:** `write`
- **Description:** `Write complete UTF-8 text to a local file. Relative paths use the working directory. Creates the file and missing parent directories, or replaces an existing regular file.`

The description states the operation and the relative-path rule without teaching patching, modes, limits, or security claims.

### JSON Schema

```json
{
  "type": "object",
  "properties": {
    "path": {
      "type": "string",
      "description": "Path to the file to create or replace (relative to the working directory or absolute)."
    },
    "content": {
      "type": "string",
      "description": "Complete text content to write."
    }
  },
  "required": ["path", "content"],
  "additionalProperties": false
}
```

### Argument validation

- Deserialize into a structure that denies unknown fields, matching the read extension's defense in depth (`extensions/read/src/lib.rs`).
- Missing fields, non-string values, and unknown fields are `invalid_arguments` with `invalid write arguments: <serde error>`.
- Reject only the exactly empty path with `invalid_arguments: path must not be empty`. Do not trim it: spaces can be meaningful path input and Windows should decide whether a particular name is legal.
- Accept every string as content, including `""`, embedded NUL characters, `\r`, `\n`, and U+FEFF. A NUL in content is ordinary UTF-8 data; a NUL in a path is rejected later by the Windows filesystem as an invalid path.
- Do not add an input-size limit. The trusted caller and JSON boundary already materialize the string, and the references provide incompatible limits ranging from 64,000 bytes to 100 MiB to no limit. No demonstrated Wren need selects a useful threshold.

## Filesystem behavior

| Destination state | Recommended behavior | Result or error |
|---|---|---|
| Missing file, existing parent | Create a new regular file | Success |
| Missing file, missing parents | Recursively create parents, then create file | Success; created parents are not rolled back after a later failure |
| Existing ordinary regular file | Open for write with create+truncate and write complete content | Success; existing file identity, ACLs, attributes, and hard links remain |
| Existing directory or other non-regular, non-reparse object | Do not open it for content replacement | `not_regular_file` |
| Existing final symlink, junction, mount point, cloud placeholder, or other reparse point | Do not follow or replace it | `unsupported_reparse_point` |
| Missing final path under an existing parent symlink/junction/reparse directory | Follow normal Windows parent resolution | Create at the resolved parent target; no containment claim |
| Existing non-directory in the parent chain | Parent creation/write cannot proceed | `invalid_destination` |
| Existing read-only or ACL-denied file | Do not clear attributes or bypass ACLs | `permission_denied`; if open failed, prior bytes remain |
| Existing file held by a no-share lock | Do not retry, rename around, or bypass the lock | `permission_denied`; prior bytes remain |
| Existing file held with compatible write sharing | Proceed under normal Windows sharing | Success or the resulting native race; Wren provides no cross-process lock |
| Empty content | Create or truncate and write no bytes | Success and a zero-byte file |
| Parent or destination disappears during the call | Do not retry an unbounded number of times | `not_found` or the native mapped error |

Relative paths are joined to `ToolContext::working_directory()`; absolute paths are used unchanged. Wren does not canonicalize for containment and does not claim that the resulting path remains under the working directory.

### Exact bytes

The resulting content is exactly `content` encoded as UTF-8:

- no BOM is added;
- a leading U+FEFF supplied in `content` becomes the UTF-8 BOM byte sequence `EF BB BF`;
- an existing BOM is discarded on replacement unless it is present in the submitted complete content;
- LF, CRLF, lone CR, and final-newline presence are preserved exactly as submitted; and
- empty content produces zero bytes.

No formatter, line-ending normalization, encoding detection, or preservation of prior bytes runs in this tool.

## Errors and output shape

### Stable `ToolError` categories

The stable contract is the kind and message prefix. The trailing Windows error text remains useful corrective detail but is not suitable for exact assertions.

| Failure class | Kind | Message template |
|---|---|---|
| Missing, wrong-type, or unknown argument | `invalid_arguments` | `invalid write arguments: <serde error>` |
| Empty path | `invalid_arguments` | `path must not be empty` |
| Final destination has `FILE_ATTRIBUTE_REPARSE_POINT` | `unsupported_reparse_point` | `<resolved path> is a reparse point; write only supports regular files` |
| Final destination exists but is not a regular file | `not_regular_file` | `<resolved path> is not a regular file` |
| Invalid Windows path/name (`InvalidInput` or raw Windows 123, 161, 206) | `invalid_path` | `could not <operation> <resolved path>: <OS error>` |
| Parent component is not a directory (`NotADirectory`, parent-creation `AlreadyExists`, or raw Windows 267) | `invalid_destination` | `could not create parent directories for <resolved path>: <OS error>` |
| Path vanished or was unavailable in a race (`NotFound`, including raw Windows 2 or 3) | `not_found` | `could not <operation> <resolved path>: <OS error>` |
| ACL, read-only, sharing, or lock denial (`PermissionDenied` or raw Windows 5, 32, 33) | `permission_denied` | `could not <operation> <resolved path>: <OS error>` |
| Disk full, device/provider failure, and all other I/O | `io` | `could not <operation> <resolved path>: <OS error>` |

`<operation>` is one of `inspect`, `create parent directories for`, `open for writing`, or `write`, identifying whether the old file could have been touched. Error mapping should align with `extensions/read/src/lib.rs` where categories overlap. Tests should assert kinds and stable prefixes, not localized OS suffixes.

A race after the destination preflight can change the apparent category—for example, a directory substituted immediately before open may surface as `permission_denied`. The first version does not claim race-free type enforcement.

### Success

Exact model-visible text:

```text
Successfully wrote <UTF-8 byte count> bytes to <supplied path>
```

The count is `content`'s UTF-8 encoded length, unlike the JavaScript-compatible counts in Pi and `pi_agent_rust`.

Structured `ToolOutput` details:

```json
{
  "path": "<supplied path>",
  "resolved_path": "<absolute resolved path for display>",
  "bytes_written": 123
}
```

Do not include a `created` or `replaced` claim. A preflight existence check cannot make that distinction race-free when the final open intentionally supports both. The current CLI writes only `ToolOutput::text()` to stdout (`src/main.rs`), so details are established by in-process tests now and are available to future orchestration through the existing extension contract.

## Recommended implementation algorithm

1. Deserialize and validate the two arguments.
2. Resolve the supplied path: leave an absolute path unchanged; otherwise join it to `context.working_directory()`.
3. Inspect the final path without following it. If it is missing, continue. If it has the Windows reparse attribute, return `unsupported_reparse_point`. If it exists and is not an ordinary file, return `not_regular_file`. Map inspection failures.
4. Recursively create the destination's parent directories. Map a non-directory parent distinctly from permission and general I/O failures.
5. Open the destination for write with create and truncate enabled. Use Rust's normal Windows sharing defaults; do not request exclusive access or special reparse processing.
6. Submit all UTF-8 bytes with `write_all`. Do not call `sync_all`, create a temporary file, or claim durability.
7. Return success text and details only after `write_all` succeeds.

This is a direct write, not an atomic replacement. It is intentionally smaller than the `pi_agent_rust` temporary-file algorithm. It also preserves the existing file object and hard-link behavior and avoids requiring Windows delete access merely to change contents.

## Race conditions and explicit guarantees

The first version provides:

- one complete in-memory UTF-8 payload submitted by one synchronous invocation;
- exact-byte behavior when the call succeeds; and
- no reported success before `write_all` completes.

It does **not** provide:

- atomic visibility to concurrent readers;
- crash, process-termination, disk-full, or power-loss atomicity;
- `fsync`-style durability;
- rollback of newly created parent directories;
- a compare-and-swap against prior contents or metadata;
- a race-free no-reparse/no-directory guarantee between preflight and open;
- serialization across Wren processes or independent extension instances;
- an advisory or mandatory file lock;
- cancellation (the extension API is synchronous and has no cancellation token); or
- a guarantee about which concurrent writer wins.

Within a single current registry, `Tool::invoke` is called synchronously through a mutable extension instance (`src/extension.rs`). No per-path queue belongs in this extension before Wren has a demonstrated concurrent invocation model.

## Packaging implications

The current installer explicitly builds and installs only `wren-read-extension`, hashes its release DLL into a generation directory, and writes an auto-load manifest (`tools/install/src/main.rs`). The functional installation helper likewise validates only read (`tools/test-support/src/install.rs`).

Issue #30 requires these narrow changes during implementation:

1. Add `extensions/write` as a workspace member with a `cdylib` package parallel to `extensions/read` (`Cargo.toml`, `extensions/read/Cargo.toml`).
2. Add `wren-write-extension` to the installer's single locked release build command.
3. Install `wren_write_extension.dll` under `bin/extensions/write/generations/<DLL hash>/` and write `bin/extensions/write/extension.toml` with `id = "write"` and `mode = "auto"`.
4. Move only the repeated DLL hashing/generation/manifest steps into a small private installer helper, called explicitly once for read and once for write. Do not add extension discovery, package metadata, dependency resolution, publishing, or a generic installer framework.
5. Extend `ReleaseInstallation::open` and cloning evidence to require both manifests and both selected generation DLLs. Keep the existing read-library accessor needed by conflict tests, and add only the corresponding write evidence needed by packaging tests (`tools/test-support/src/install.rs`, `tests/functional.rs`).

No extension API revision is required: name, description, schema, `ToolContext`, `ToolOutput`, and `ToolError` already express the full contract (`crates/wren-extension/src/lib.rs`).

## Test and evaluation plan

### Unit tests

Keep unit tests beside the write extension, following the read extension's pattern (`extensions/read/src/lib.rs`). The narrow matrix is:

1. **Definition and arguments**
   - exact name, description, and schema;
   - required fields and string types;
   - unknown-field rejection;
   - empty-path rejection;
   - empty content acceptance.
2. **Resolution and state**
   - relative resolution from `ToolContext::working_directory()`;
   - absolute path unchanged;
   - new file and recursively nested parents;
   - existing regular file replaced and a shorter replacement truncates old bytes.
3. **Exact bytes and output**
   - non-ASCII UTF-8 byte count;
   - explicit U+FEFF, CRLF, lone LF, and no final newline remain exact;
   - empty content produces a zero-byte file;
   - success text and details contain supplied path, resolved display path, and actual byte count.
4. **Deterministic failures**
   - directory destination;
   - non-directory parent;
   - final reparse-point classification through a non-elevated directory-junction fixture or a focused Windows metadata helper test;
   - I/O mapping for `PermissionDenied`, raw Windows 5/32/33, invalid path, not found, and generic I/O.

Do not allocate a huge string merely to test the intentional absence of a size limit. Do not simulate atomicity or agent behavior.

### Installed-release functional tests

Use the existing `cargo test --test functional` entrypoint, one complete optimized `cargo install-wren` installation, cloned per scenario. Every scenario uses an isolated workspace and a unique `WREN_HOME`; neither is called a security sandbox (`tests/functional.rs`, `docs/principles/testing.md`).

Representative production-boundary coverage:

1. **Packaging and regression:** the installed release contains selected generation DLLs/manifests for read and write; both auto-load; startup succeeds; the installed read tool still works.
2. **Create:** a relative nested path creates missing parents and exact bytes containing non-ASCII text, explicit BOM, CRLF, and a missing final newline.
3. **Replace:** an absolute path replaces a longer existing file with a shorter payload and leaves exactly the submitted bytes.
4. **Empty:** empty content creates or truncates to a zero-byte file.
5. **Arguments:** malformed JSON plus representative missing, wrong-type, unknown, and empty-path calls fail nonzero with `invalid_arguments`, empty stdout, and a stable stderr prefix.
6. **Destinations:** a directory and a parent-file conflict return the designed kinds. A final directory junction made through an unattended, non-elevated Windows fixture establishes `unsupported_reparse_point`; no test may depend on Developer Mode, UAC, or manual setup.
7. **Windows denial:** hold an existing destination with `share_mode(0)`, invoke installed write, assert `permission_denied`, empty stdout, and unchanged old bytes. Mark a second existing file read-only, assert the same observable result, and restore the attribute before cleanup.
8. **Environment:** relative and absolute scenarios explicitly establish cwd resolution and unique `WREN_HOME` use rather than relying only on helper implementation.

The current CLI does not expose structured details, so details remain unit evidence. Do not add a new CLI output mode solely to make this functional assertion.

Before implementation work is considered complete, the future implementing agent must run all credentialless repository gates and the complete optimized installation required by `AGENTS.md` and `.agents/skills/wren-testing/SKILL.md`. No authenticated model call is needed for this local tool.

### Later behavioral evaluation under #29

Direct tests can establish schema serialization, dispatch, path behavior, bytes, errors, and packaging. They cannot establish:

- whether a real model selects `write` when appropriate;
- whether the description and field wording are effective;
- how reliably it constructs exact complete content and paths;
- whether parent creation improves task completion;
- whether the success/error wording produces efficient correction;
- tool-call count, token use, latency, or reliability on the shared `exact-file-edit` task; or
- future choice between whole-file write and a separate edit capability.

Those are behavioral claims. Evaluate them later through #29 with repeated paired or interleaved real-agent attempts, fixed harness/model/provider/reasoning/task/budgets, deterministic artifact verification, and preserved transcripts/results as required by `docs/principles/testing.md`. A direct tool invocation or one successful agent attempt is not effectiveness evidence.

## Rejected alternatives

| Alternative | Reason rejected for #30 |
|---|---|
| Exact replacement, patch, hashline, or multi-file edit | Explicitly outside issue scope; separate behavior and evaluation questions |
| Append, skip, fail-if-exists, encoding, or ranged-write modes | Expands schema and model choices without a demonstrated baseline need |
| Cwd containment or rejection of parent junctions | Wren extensions are trusted capabilities, not a sandbox; absolute paths are required |
| Follow or replace a final reparse point | Ambiguous and provider-dependent; reject it deterministically instead |
| Same-directory temporary file plus rename/`ReplaceFileW` | Requires Windows decisions about delete sharing, ACL/attribute transfer, hard links, reparse points, cleanup, and exceptional replacement states; no demonstrated need outweighs the complexity |
| Delete destination then rename temporary file | Creates a missing-name window and can lose the old file; weaker than the direct baseline |
| Preserve an existing BOM or line-ending style | Contradicts complete-content replacement and exact submitted-byte behavior |
| Formatter or LSP writethrough | Makes a successful write differ from submitted content |
| 64 KiB or 100 MiB input cap copied from a reference | Reference thresholds conflict and no Wren requirement selects one |
| Per-file mutation queue or optimistic conflict snapshot | Current extension invocation is synchronous; future orchestration should first demonstrate concurrent mutation needs |
| Cancellation protocol | Not present in API revision 2 and cannot be added by this extension alone |
| Generic bundled-extension installer framework | Two known bundled DLLs need only one narrow repeated helper, not a new packaging abstraction |

## Limitations and unresolved questions

There are no blocking product decisions left for implementing issue #30 under this recommendation. The following limitations are explicit rather than unresolved implementation choices:

- direct writes can truncate or partially write on a late failure;
- success is not a durability guarantee;
- parent directories can remain after a failed file write;
- parent reparse points are followed and may lead outside cwd;
- final reparse enforcement has a time-of-check/time-of-use race and is not a security boundary;
- hard-linked names observe replacement because file identity is retained;
- provider-specific Windows paths, network filesystems, alternate data streams, and reserved names receive native behavior outside the explicitly mapped categories;
- a non-Unicode resolved Windows path can only be represented lossily in JSON details, while the supplied JSON path remains exact;
- compatible-share concurrent writers can interleave or overwrite one another; and
- structured details are not displayed by the current direct-invocation CLI.

If later evidence requires crash-atomic replacement, concurrency control, size backpressure, path containment, or richer path namespaces, each should be scoped from that evidence rather than preloaded into the baseline write tool.
