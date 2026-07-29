# Provider extension design

> **Issue:** [#31 — Add provider extensions with OpenAI support](https://github.com/learywood/wren/issues/31)
>
> **Status:** Architecture reopened. The native async-runtime direction below is not approved for implementation; see [Provider host-services research](provider-host-services-research.md).

The original provider contract and Pi-alignment findings remain useful historical rationale, but its async HTTP assumption is superseded by the completed [Windows host-services spike](provider-host-services-research.md#windows-native-spike). Production implementation is paused pending a revised design approval.

## Scope and evidence boundary

Issue #31 establishes providers as production extension capabilities and implements the OpenAI provider needed by #29. It ends at the provider boundary.

It includes:

- provider registration, discovery, metadata validation, selection, conflicts, and lifecycle through the native extension registry;
- a provider-neutral asynchronous streaming contract modeled on Pi's capability;
- cancellation, timeout, assistant text, reasoning replay state, tool calls and correlated results, usage, and provider errors;
- an ordinary bundled OpenAI extension using the Responses API, `gpt-5.6-sol`, low reasoning, and environment-provided `OPENAI_API_KEY`;
- controlled endpoint tests, installed discovery evidence, and a narrow authenticated extension-contract smoke; and
- startup performance evidence for the bundled extension.

It excludes `wren exec`, the agent/tool loop, evaluator integration, persistent sessions, credential management, and behavioral claims. Those remain in #29.

A provider is an internal model-service adapter, not a user-invoked command or a tool. The future agent runtime will use it to translate context and tools into a provider protocol and stream model output back. Wren will not expose a raw `wren provider invoke` command merely to create a test seam.

Wren was inspected at `1bd0fcd0e653852a1653888170173f6c54ca73cb`. The current API revision 2 contract exposes indexed tools from a long-lived extension instance. The registry copies registration names, borrows capabilities only while the owning library remains loaded, destroys extension-owned state before unloading its library, and installs generation-specific DLLs beside `wren.exe` (`crates/wren-extension/src/lib.rs`, `src/extension.rs`, `tools/install/src/main.rs`). Providers should extend these ownership and discovery rules rather than bypass them.

## Capability reference and other sources

Pi is the capability reference for Wren providers. Wren's Rust and native-DLL implementation will differ, but the provider-visible context, asynchronous stream behavior, cancellation, tool correlation, reasoning replay, and usage semantics should follow Pi unless a documented Wren constraint requires otherwise.

The synchronized Pi checkout inspected is:

| Repository | Revision | Relevant implementation |
|---|---|---|
| `earendil-works/pi` | `c820aa26fe0907e053e881a957722693fc094c9c` | `packages/ai/src/{models.ts,types.ts}`, `packages/ai/src/providers/openai.ts`, `packages/ai/src/api/{openai-responses.ts,openai-responses-shared.ts,lazy.ts}`, `packages/ai/test/abort.test.ts`, provider extension registration under `packages/coding-agent/src/core/` |

Additional references were inspected for Rust/OpenAI implementation evidence:

| Repository | Revision | Relevant implementation |
|---|---|---|
| `openai/codex` | `f029bb795ccbbd8471511f5a8b93e56d8f2b6d31` | `codex-rs/codex-api/src/{common.rs,error.rs,endpoint/responses.rs}`, `codex-rs/protocol/src/models.rs` |
| `0xPlaygrounds/rig` | `fb3b347d884babb2dabd7704a94a72c8e7efb09b` | `crates/rig-core/src/providers/openai/{client.rs,responses_api/mod.rs}`, OpenAI cassette/live tests |

First-party OpenAI documentation inspected on 2026-07-28:

- [Create a model response](https://developers.openai.com/api/reference/resources/responses/methods/create)
- [Function calling](https://developers.openai.com/api/docs/guides/function-calling)
- [Reasoning models](https://developers.openai.com/api/docs/guides/reasoning)
- [GPT-5.6 Sol](https://developers.openai.com/api/docs/models/gpt-5.6-sol)
- [Error codes](https://developers.openai.com/api/docs/guides/error-codes)

## Findings from Pi

### Provider capability

Pi treats a provider as a concrete runtime unit identified by provider ID. It owns authentication semantics, model metadata, and stream behavior. Its runtime resolves the selected provider and delegates a model request to that provider. Extensions can register providers as first-class capabilities.

Wren needs only the subset demonstrated by #29: provider identity and streaming invocation for an explicitly selected model. A model catalog, pricing table, login flow, compatibility framework, and dynamic model refresh are not required by issue #31.

### Complete context and stateless replay

Pi passes a complete provider-neutral context on every request:

- system prompt;
- user messages;
- previous assistant content;
- tool calls;
- correlated tool results; and
- current tool definitions.

For OpenAI Responses, Pi uses `store: false`, requests `reasoning.encrypted_content`, stores opaque reasoning items with assistant content, and replays them on later requests. It also preserves OpenAI assistant message IDs and `commentary`/`final_answer` phases.

Wren will follow this behavior. The earlier proposal to make `previous_response_id` and server-stored continuation central to the common contract is rejected. Provider-specific replay metadata remains opaque to orchestration but travels with the provider-neutral assistant history.

### Ordered content and tool correlation

Pi preserves ordered assistant content containing reasoning, text, and tool calls. Final function arguments are parsed to an object before orchestration sees the tool call. Tool results carry the original opaque tool-call identity, tool name, content, and an error flag.

Wren should preserve the same properties:

- ordered output rather than separate text and tool-call collections;
- assistant text phase and opaque replay metadata;
- opaque reasoning replay data;
- JSON-object function arguments, with invalid final JSON treated as malformed provider output; and
- correlated tool results with `is_error`.

### Async streaming, timeout, and cancellation

Pi's provider returns an event stream immediately and performs asynchronous setup and network work behind it. Its stream emits start, content deltas, completed content, and one terminal success or error. Every provider call receives an abort signal. Pi tests immediate and in-flight OpenAI cancellation and preserves a distinct aborted terminal outcome.

Wren will build asynchronous streaming into the provider system now rather than introduce a synchronous contract that #29 would later replace. Timeout and cancellation must interrupt active network and stream work, not merely be checked before and after a blocking request.

### Usage semantics

Pi reports:

- uncached input tokens;
- cache-read tokens;
- cache-write tokens;
- output tokens;
- reasoning tokens as a subset of output; and
- provider-reported total tokens.

OpenAI includes cached tokens in `input_tokens`, so Pi subtracts cache-read and cache-write values when populating ordinary input. Wren should use the same semantics. Cost calculation remains outside #31 because it requires model pricing metadata.

### Lazy implementation work

Pi registers the provider but lazily imports its API implementation on first request. Authentication and network setup also occur at request time.

Wren's OpenAI extension should initialize without reading credentials, creating a runtime/client, or accessing the network. Whether merely mapping the native DLL at startup is acceptably cheap must be established by the required startup comparison.

## Decided provider contract direction

The extension API revision must increase because the native Rust ABI changes. Existing tools remain synchronous. Providers are a separate indexed capability:

```text
Extension::provider(index) -> Option<&mut dyn Provider>
Provider::definition() -> ProviderDefinition
Provider::stream(request, context) -> owned ProviderStreamInstance
```

The exact Rust polling surface remains an implementation-design detail, but it must have these properties:

- Wren owns the async executor and polls the provider stream;
- the OpenAI extension does not create a runtime and synchronously block on it;
- the stream is extension-owned and can outlive the brief mutable borrow used to create it;
- the registry retains the owning DLL until the stream is destroyed;
- dropping the stream cancels its active provider work;
- cancellation wakes a pending stream promptly;
- timeout is enforced asynchronously through terminal completion; and
- multiple future streams are not precluded by a synchronous mutable borrow held for their duration.

The likely native representation is an owned extension-created stream object with explicit poll and destroy functions, analogous to `ExtensionInstance`. This avoids exposing an unstable executor-specific future type across the DLL boundary while still allowing Wren's executor to drive it.

### Request context

The common request contains:

- non-empty provider-selected model ID;
- reasoning level;
- optional system/developer prompt;
- complete ordered message history; and
- current function-tool definitions.

Message history contains:

- user text;
- assistant content; and
- tool results.

Assistant content contains:

- text with optional `commentary`/`final_answer` phase and opaque replay metadata;
- opaque reasoning state with optional model-visible summary text; and
- a tool call with opaque ID, non-empty name, and JSON-object arguments.

A tool result contains the unchanged opaque call ID, tool name, text output, and `is_error`. Text and function tools are sufficient for #29; images, custom grammars, built-in provider tools, and deferred tool loading remain outside #31.

### Stream events

The provider stream follows Pi's lifecycle semantics:

```text
start
text_start / text_delta / text_end
thinking_start / thinking_delta / thinking_end
toolcall_start / toolcall_delta / toolcall_end
done
error
```

Requirements:

- content indexes preserve provider output order;
- `*_end` events carry complete finalized content and replay metadata;
- `toolcall_end` carries parsed object arguments;
- `done` carries the successful stop reason and final usage;
- `error` carries `aborted`, `timeout`, or provider failure and any safely retained partial content;
- exactly one terminal event is emitted; and
- ending without a terminal event is a malformed provider stream.

This gives #29 the Pi-like capability to forward deltas into its own JSONL lifecycle without changing the provider ABI.

### Errors

Stable error categories should include:

- `invalid_request`;
- `authentication`;
- `timeout`;
- `aborted`;
- `transport`;
- `provider`; and
- `malformed_response`.

Once a valid stream is returned, request/model/runtime failures terminate through the stream error protocol, matching Pi. Registry selection and metadata failures occur before a provider stream exists.

Do not expose the Authorization header or arbitrary raw response bodies. Parse documented provider errors, bound included text, mark authorization headers sensitive, and redact the exact credential from every returned error path.

## Registry and lifecycle decisions

Extend `ExtensionRegistry` with a provider-owner map parallel to the tool-owner map.

Loading remains atomic:

1. initialize the extension;
2. validate all tool and provider metadata;
3. reject duplicate names within the extension;
4. reject provider names already owned by another loaded extension;
5. publish registrations only after every conflict check succeeds; and
6. retain the extension instance and library for the process lifetime.

Duplicate provider names are rejected rather than silently overriding an existing provider. This follows Wren's deterministic registry behavior even though Pi supports explicit provider replacement.

Provider and tool names occupy separate namespaces. Invocation locates the owning extension, reacquires the provider by stable index, verifies unchanged metadata, creates an owned stream, and releases the provider borrow. The active stream retains the DLL independently until its extension-owned state is destroyed.

## OpenAI extension direction

- **Extension ID/name:** `openai`
- **Provider name:** `openai`
- **Endpoint:** `https://api.openai.com/v1/responses`
- **Authentication:** `OPENAI_API_KEY`, resolved only for a request
- **Initialization:** no credential read, runtime/client creation, or network access
- **Transport:** Responses API server-sent event stream
- **State:** `store: false` with encrypted reasoning replay

Wire mapping:

- common model -> `model`;
- low reasoning -> `reasoning.effort = "low"`;
- complete messages and tool results -> Responses `input` items;
- functions -> Responses function tools;
- opaque prior reasoning -> replayed reasoning items;
- `function_call` -> ordered tool-call stream events;
- `function_call_output` -> correlated result by original call ID;
- message ID and phase -> opaque replay metadata plus common phase;
- response usage -> Pi-compatible normalized usage; and
- terminal Responses events -> one Wren terminal stream event.

Unknown stream events and output item variants should be ignored when safe for forward compatibility. Missing required fields, invalid finalized arguments, contradictory indexes/types, or a stream ending without a terminal Responses event are malformed responses.

A production base-URL override is not proposed. Controlled tests may inject a loopback endpoint privately. Other OpenAI-compatible services should be separate provider extensions rather than changing the identity of the bundled OpenAI provider.

## Installation and internal consumption

The OpenAI provider is an ordinary bundled extension installed under:

```text
bin/extensions/openai/
  extension.toml
  generations/<generation>/wren_openai_extension.dll
```

`cargo install-wren` should build it in the same locked optimized build as Wren, read, and write. No provider-specific directory, static harness integration, or feature-specific loader is introduced.

There will be no raw provider-invocation CLI. The future `wren exec` implementation in #29 is the user-facing consumer and will select and stream the provider internally through the registry.

To make the same production registry code testable without inventing a user command, Wren's registry should live in reusable production library code used by the thin `wren.exe` binary. Release integration tests can use that same registry implementation to load and stream an installed release extension. This is a test host for the extension boundary, not an alternate provider implementation or shipped command.

## Credentials and process environments

Normal credentialless tests and installer/build children must explicitly remove `OPENAI_API_KEY`. A separate authenticated test policy supplies it only to the process hosting the real OpenAI extension stream. Verifier and unrelated child environments remain credential-free.

Never place the key in:

- request objects or command arguments;
- Wren configuration or extension manifests;
- stdout/stderr, panic output, or assertion messages;
- captured request/response fixtures;
- issue/PR comments or test artifacts; or
- unrelated child processes.

Controlled endpoint tests use a sentinel credential and assert that it is absent from errors and artifacts. They may inspect the Authorization header in memory only; they must not persist it.

## Acceptance evidence

### Contract and registry

Deterministic fixture coverage should establish:

- provider registration and name selection;
- empty, duplicate-within-extension, changed, and cross-extension provider names;
- extension and active-stream lifecycle while the DLL remains loaded;
- ordered text, reasoning, and tool-call events;
- parsed tool arguments and correlated error/success tool results;
- normalized usage and terminal stop reasons;
- provider errors and malformed stream termination;
- pre-cancelled and in-flight cancellation; and
- timeout distinct from cancellation.

### Controlled OpenAI endpoint

A loopback SSE server should establish:

- exact path, method, content type, bearer authentication, model, low reasoning, `store: false`, tools, complete replay history, encrypted reasoning, and correlated function output construction;
- ordered text/reasoning deltas and multiple function calls;
- assistant IDs/phases and replay metadata;
- all usage fields with Pi-compatible normalization;
- unknown-event compatibility;
- malformed JSON, malformed known events, and missing terminal events;
- HTTP and streamed provider errors;
- missing and empty authentication;
- timeout and cancellation dropping active network work; and
- credential redaction.

These tests exercise the real OpenAI implementation against a controlled endpoint without making a production-provider claim.

### Installed credentialless path

The complete optimized installation should prove that:

- the selected OpenAI generation DLL and ordinary manifest are installed;
- `wren.exe` discovers and validates it through normal startup;
- startup requires no authentication or network access;
- provider conflicts and malformed metadata fail through the real registry; and
- credentialless build/test children do not inherit a developer key.

### Authenticated smoke

An explicitly requested ignored release integration test should:

- use the production registry implementation;
- load the installed release OpenAI DLL with the matching native fingerprint;
- run its real async stream against the production endpoint;
- select `gpt-5.6-sol` and low reasoning;
- use a constrained prompt requiring one fixed token;
- assert the expected finalized text, no tool calls, exactly one successful terminal event, and internally consistent positive usage; and
- record only nonsecret model, reasoning, duration, event counts, and usage.

This proves that the provider extension boundary worked once. It does not claim that `wren.exe` can yet run an agent; #29 will establish that complete production-process claim through `wren exec`.

### Final gates

Before merge:

- formatting and linting;
- all unit and credentialless functional tests;
- evaluator validation and repository CI-equivalent checks;
- `cargo install-wren`;
- the local authenticated release smoke;
- baseline/candidate installed startup comparison; and
- issue/PR attachment of nonsecret authenticated and performance evidence.

## Rejected alternatives

| Alternative | Reason rejected |
|---|---|
| Build OpenAI directly into Wren | Violates the extension principle and cannot prove provider discovery/conflicts |
| Treat a provider as a tool or user-invoked command | Providers are internal agent-runtime adapters; a raw command has no product utility |
| Add a raw `wren provider invoke` command for testing | Confuses a test seam with product API and would be unused by #29 |
| Synchronous provider invocation | Wren is expected to reach Pi-like async streaming; a synchronous ABI would be replaced immediately |
| Server-stored continuation as the common contract | Pi uses complete context, `store: false`, and opaque encrypted reasoning replay |
| Hold a mutable provider borrow for the stream lifetime | Prevents a clean owned async stream and complicates concurrency and DLL ownership |
| Blocking HTTP worker abandoned on timeout | Returns before provider work actually stops and weakens lifecycle guarantees |
| Silent duplicate-provider replacement | Wren requires deterministic conflict handling |
| Production `OPENAI_BASE_URL` override | Expands scope to compatible providers and can redirect credentials |
| Strict OpenAI function schemas | Existing Wren schemas contain optional properties and are not all strict-mode compatible |
| Authentication during extension initialization | Makes startup require credentials and prevents credentialless discovery evidence |
| Treat missing credentials as skipped/passing authentication evidence | Violates repository testing policy |
| Add `wren exec` in #31 | The agent command and tool loop remain together in #29 |

## Superseded approval checkpoint

This checkpoint is retained as history. The native polling and runtime direction was reopened on 2026-07-29; the host-services spike findings and current unresolved work are recorded in [Provider host-services research](provider-host-services-research.md).

The following directions were explicitly decided in the earlier design review:

- Pi is the provider capability reference.
- The provider system is asynchronous and streaming from its first revision.
- Requests carry complete context and OpenAI uses stateless encrypted reasoning replay rather than a continuation-only contract.
- Duplicate provider names are rejected.
- Providers are internal capabilities; no raw provider-invocation CLI is added.

These earlier open details are not sufficient for implementation approval. The host-services boundary, async ABI, lifecycle, provider compatibility, and revised acceptance evidence must now be resolved first.
