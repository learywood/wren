# Provider extension design

## Scope and evidence boundary

This document recommends the smallest provider extension contract and OpenAI implementation that satisfy issue #31. It ends at provider invocation. `wren exec`, agent orchestration, tool execution, evaluator integration, sessions, credential management, and behavioral claims remain in #29.

The recommendation is:

- add providers as a second capability exposed by the existing trusted native extension mechanism;
- bundle one ordinary auto-loaded extension whose extension ID and provider name are both `openai`;
- expose one generic provider-boundary command for installed functional evidence, without adding agent behavior;
- use the OpenAI Responses API at `https://api.openai.com/v1/responses` with environment-provided `OPENAI_API_KEY`;
- carry assistant text, function calls, correlated function results, continuation, and token usage through provider-neutral native types;
- support explicit deadlines and cancellation at the provider call boundary; and
- establish production OpenAI behavior with one narrow local authenticated smoke, not a behavioral evaluation.

Wren was inspected at `1bd0fcd0e653852a1653888170173f6c54ca73cb`. The current API revision 2 contract exposes indexed tools from a long-lived extension instance. The registry copies registration names, borrows capabilities only while the owning library remains loaded, destroys extension-owned state before unloading its library, and installs generation-specific DLLs beside `wren.exe` (`crates/wren-extension/src/lib.rs`, `src/extension.rs`, `tools/install/src/main.rs`). These ownership and discovery rules are suitable for providers and should be extended rather than bypassed.

## Sources inspected

First-party OpenAI documentation was inspected on 2026-07-28:

- [Create a model response](https://developers.openai.com/api/reference/resources/responses/methods/create)
- [Function calling](https://developers.openai.com/api/docs/guides/function-calling)
- [Reasoning models](https://developers.openai.com/api/docs/guides/reasoning)
- [GPT-5.6 Sol](https://developers.openai.com/api/docs/models/gpt-5.6-sol)
- [Error codes](https://developers.openai.com/api/docs/guides/error-codes)

The following reference implementations were synchronized outside the Wren repository as prescribed by `docs/references/github.com`:

| Repository | Revision inspected | Relevant implementation |
|---|---|---|
| `openai/codex` | `f029bb795ccbbd8471511f5a8b93e56d8f2b6d31` | `codex-rs/codex-api/src/{common.rs,error.rs,endpoint/responses.rs}`, `codex-rs/protocol/src/models.rs` |
| `0xPlaygrounds/rig` | `fb3b347d884babb2dabd7704a94a72c8e7efb09b` | `crates/rig-core/src/providers/openai/{client.rs,responses_api/mod.rs}`, OpenAI cassette/live tests |
| `earendil-works/pi` | `c820aa26fe0907e053e881a957722693fc094c9c` | `packages/ai/src/api/{openai-responses.ts,openai-responses-shared.ts}`, `types.ts`, abort tests |

## Findings

### Responses protocol

OpenAI documents a Responses request with `model`, `input`, optional `reasoning`, and optional function `tools`. A function call output item contains a JSON-encoded `arguments` string, `name`, and `call_id`. The next request correlates the local result with the same `call_id` in a `function_call_output` input item. A response can contain zero, one, or multiple calls.

Reasoning models require prior reasoning and call items to remain available when function results are submitted. The API supports this either by replaying every prior output item or by passing `previous_response_id`. The latter is the narrowest provider boundary because provider-specific reasoning payloads do not have to leak into Wren's common contract. Wren should therefore call the provider again with an opaque continuation string returned by the prior call. The OpenAI extension maps that string to `previous_response_id`.

The OpenAI documentation identifies `gpt-5.6-sol` as a Responses-capable reasoning and function-calling model. `reasoning.effort = "low"` is documented; reasoning effort and reasoning mode are independent, and standard mode is represented by omitting `mode`. Issue #31 requires low effort, not pro mode.

Responses usage includes input, output, total, cached-input, and reasoning token counts. Reasoning tokens are part of output tokens, so totals must be preserved as reported rather than recomputed by summing every detail field.

A completed Responses object can contain assistant message items and function call items. Assistant text is in `message.content[]` entries of type `output_text`. Unknown output item types should be ignored for forward compatibility, but known item types with missing or invalid required fields make the response malformed. A non-`completed` top-level status is not a successful provider response.

### Reference implementations

Codex uses typed Responses request data, keeps function arguments as the raw JSON string returned by the API, carries function call outputs by `call_id`, records separate usage details, and applies an explicit stream idle timeout. Its HTTP and stream errors are typed rather than exposing arbitrary response bodies.

Rig converts provider-specific output into common assistant text, tool calls, reasoning, and usage. Its implementation and tests demonstrate that reasoning items must round-trip for stateless history, that unknown output variants should not discard otherwise valid usage, and that missing optional usage detail objects can be tolerated. Rig reads `OPENAI_API_KEY` through its provider client constructor and keeps live tests explicitly ignored unless authentication is requested.

Pi gives every provider call an abort signal, tests immediate and in-flight OpenAI cancellation, combines caller cancellation with timeout signals, preserves raw function arguments and correlated results, and normalizes provider usage. Its generic framework is intentionally broader than Wren needs here; Wren should copy the cancellation and correlation properties, not Pi's full provider/model catalog.

### HTTP implementation

Reqwest 0.13.4 supports a total request deadline and marks sensitive headers. Dropping its async request/body future cancels the in-flight operation. A small current-thread Tokio runtime inside the OpenAI extension can synchronously satisfy Wren's native trait while selecting among request completion, the host deadline, and the host cancellation signal. This is narrower and more truthful than running a blocking request on an abandoned worker thread.

The production endpoint must be a constant. A private test constructor may inject a loopback endpoint for controlled tests, but the production extension should not honor a base-URL environment variable in this issue. That prevents an unrelated environment value from redirecting the API key and avoids silently creating a general OpenAI-compatible-provider feature.

## Recommended native contract

Increment the extension API revision because the native Rust ABI changes. Keep the existing `Extension` and `Tool` behavior and add:

```text
Extension::provider(index) -> Option<&mut dyn Provider>
Provider::definition() -> ProviderDefinition
Provider::invoke(request, context) -> Result<ProviderResponse, ProviderError>
```

Provider indexes are contiguous and stable after initialization, exactly like tool indexes. `ProviderDefinition` contains only a non-empty provider name. No model catalog, pricing table, credential callback, configuration schema, or provider-specific options belong in this revision.

### Request

The common request contains:

- non-empty `model`;
- optional documented reasoning effort;
- one or more input items;
- zero or more function definitions; and
- optional opaque continuation returned by the same provider.

Input items are only:

- a text message with `developer` or `user` role; or
- a function result with non-empty `call_id` and string output.

A function definition owns its name, description, and object JSON Schema. The provider contract preserves function arguments as a raw string in responses; #29 will parse and validate that string before invoking a tool.

The first #29 request will carry developer/user text and no continuation. A follow-up request will carry correlated function results and the prior continuation. The provider owns translation into its wire protocol.

### Response

The common response contains:

- a non-empty opaque continuation string;
- ordered output items;
- reported usage.

Output items are assistant text or a function call with non-empty `call_id`, name, and raw arguments. Keeping an ordered list preserves commentary around calls without defining streaming. Usage contains input, cached input, output, reasoning, and total token counts.

### Context, timeout, and cancellation

`ProviderContext` contains a positive timeout and a clonable cancellation signal. The host owns the corresponding cancellation handle. Providers must:

1. reject an already-cancelled call without starting work;
2. stop polling and drop in-flight network work when cancellation wins;
3. return `cancelled` distinctly from `timeout`; and
4. apply the timeout through receipt and parsing of the complete response body.

The context is a call boundary, not a scheduler. Wren does not add retries, backoff, streaming, concurrency, or background work in this issue.

### Errors

`ProviderError` has a stable kind and corrective message. Required kinds are:

- `invalid_request` — malformed common request;
- `authentication` — missing/empty `OPENAI_API_KEY` or an authentication rejection;
- `timeout` — provider call deadline elapsed;
- `cancelled` — host cancellation won;
- `transport` — DNS, TLS, connection, or body transport failure;
- `provider` — a well-formed non-success HTTP/API response or non-completed response status; and
- `malformed_response` — success HTTP status with an invalid Responses object.

Do not expose the Authorization header or raw error body. Parse the documented `error.message`, status, and safe code when present, bound any included text, and redact the exact credential from every error path before returning it. Reqwest Authorization header values must be marked sensitive.

## Registry and lifecycle

Extend `ExtensionRegistry` with a provider-owner map parallel to the tool-owner map.

Loading remains atomic:

1. initialize the extension;
2. validate all tool and provider metadata;
3. reject duplicate names within the extension;
4. reject names already owned by another loaded extension;
5. publish all names only after every conflict check succeeds; and
6. retain the extension instance and library for the process lifetime.

Provider names conflict only with provider names; an extension and capability may both be named `openai`. Invocation locates the owning extension, reacquires the provider by its stable index, verifies that its definition has not changed, and calls it while the library remains loaded. The registry must not retain self-referential provider borrows.

The fixture extension should expose a controlled provider in addition to its existing conflict tool. Contract and registry tests should cover successful invocation, structured provider failure, timeout, registration changes, and destruction while the DLL remains loaded. Empty and duplicate provider names cover malformed metadata and conflict behavior.

## Installed command boundary

Add one generic provider diagnostic boundary:

```text
wren provider <name> --request <json> [--timeout-ms <positive integer>]
```

It parses the common request, starts the real installed registry, invokes the selected provider, and writes the common response as one JSON object. Errors go to stderr with a nonzero exit status. The default timeout should be explicit and documented in code; this command performs one provider call only.

This is not `wren exec`: it has no stdin prompt contract, tool execution, loop, session, event protocol, model defaults, or evaluator behavior. It exists to prove provider selection and invocation through the complete installed boundary. #29 will call the registry directly rather than shelling out to this command.

## OpenAI extension

- **Extension ID/name:** `openai`
- **Provider name:** `openai`
- **Loading:** bundled and `auto`
- **Endpoint:** fixed `https://api.openai.com/v1/responses`
- **Authentication:** `OPENAI_API_KEY`, read only when invoking
- **Initialization:** no credential read and no network access

Wire mapping:

- common `model` -> `model`;
- common reasoning effort -> `reasoning.effort`;
- messages/results -> Responses `input` items;
- common functions -> Responses function tools with `strict: false` so Wren's existing schemas with optional properties remain valid;
- continuation -> `previous_response_id`;
- fixed `store: true` -> server-side continuation for reasoning/tool turns;
- response ID -> common continuation;
- `message/output_text` -> assistant output;
- `function_call` -> common call preserving raw arguments;
- documented usage -> common usage.

The parser should deny invalid known shapes while tolerating unknown output variants. It must reject missing/empty response ID, missing usage, non-completed status, malformed assistant content, and malformed function calls.

## Credentials and process environments

Normal credentialless tests and installer/build children must explicitly remove `OPENAI_API_KEY`; inheriting the developer's key into a test that does not need it is not acceptable. Add a separate test-support environment policy that passes the key only to the installed Wren process used by the authenticated smoke. Verifier and unrelated child policies remain credential-free.

Never place the key in:

- request JSON or command arguments;
- Wren configuration or extension manifests;
- stdout/stderr, panic output, or assertion messages;
- captured request/response fixtures;
- issue/PR comments or test artifacts; or
- child processes that do not invoke OpenAI.

Controlled endpoint tests should use a sentinel credential and assert that it is absent from provider errors and captured artifacts. They may inspect the Authorization header in memory only to verify request construction; they must not persist it.

## Installation decision

The OpenAI provider is a bundled ordinary extension. `cargo install-wren` should explicitly build `wren-openai-extension` in the same locked optimized build as Wren, read, and write, then install its generation DLL and ordinary manifest under:

```text
bin/extensions/openai/
  extension.toml
  generations/<generation>/wren_openai_extension.dll
```

No provider-specific directory, harness lookup, feature flag, or static linkage is introduced. `ReleaseInstallation::open` should require the selected OpenAI DLL just as it requires read and write. Installed startup must auto-load all three extensions without authentication or network access.

## Acceptance evidence

### Contract and registry

Use deterministic tests for:

- provider registration and name selection;
- empty, duplicate-within-extension, changed, and cross-extension provider names;
- extension lifecycle and provider invocation while its DLL remains loaded;
- successful assistant text, tool calls, continuation, and usage;
- structured provider errors;
- pre-cancelled and in-flight cancellation; and
- deadline timeout distinct from cancellation.

### Controlled OpenAI endpoint

A loopback HTTP server should establish:

- exact path, method, content type, bearer authentication, model, low reasoning, messages, tools, continuation, and correlated function outputs;
- ordered assistant text and multiple function-call parsing;
- all usage fields;
- unknown output compatibility;
- malformed JSON and malformed known items;
- HTTP errors and documented provider error extraction;
- missing and empty authentication;
- timeout and cancellation dropping an in-flight request; and
- secret redaction.

These tests exercise the real OpenAI implementation against a controlled endpoint and make no production-provider claim.

### Installed credentialless path

`cargo test --test functional` should prove that the complete optimized installation contains and auto-loads the OpenAI extension, selects providers through the registry, handles fixture conflicts/errors/timeouts, and reports missing authentication without contacting a network endpoint. The default credentialless path must scrub any developer `OPENAI_API_KEY`.

### Authenticated smoke

Run one explicitly requested ignored/local test through the installed `wren.exe`, ordinary extension discovery, provider registry, real OpenAI extension, production endpoint, `gpt-5.6-sol`, and low reasoning. Use a constrained prompt requiring one fixed token and assert the exact assistant text, no tool calls, a non-empty continuation, and internally consistent positive usage.

The smoke record contains only command identity, model, reasoning, pass/fail, duration, and nonsecret usage. It proves that path worked once. It does not establish agent orchestration, reliability, parity, or tool use.

### Final gates

Before merge:

- format and lint;
- all unit and credentialless functional tests;
- evaluator validation and repository CI-equivalent checks;
- `cargo install-wren`;
- the local authenticated smoke;
- baseline/candidate installed startup comparison; and
- issue/PR attachment of nonsecret authenticated and performance evidence.

## Rejected alternatives

| Alternative | Reason rejected |
|---|---|
| Build OpenAI directly into Wren | Violates the extension principle and cannot prove provider discovery/conflicts |
| Provider-specific harness lookup or manifest | Creates a privileged provider path |
| General model catalog, pricing, retries, streaming, or credential manager | Not required by #29's provider boundary |
| Replay arbitrary OpenAI reasoning JSON in the common contract | Leaks provider wire details; continuation is smaller |
| Blocking HTTP worker abandoned on timeout | Returns before extension-owned work actually stops and weakens lifecycle guarantees |
| Production `OPENAI_BASE_URL` override | Expands scope to compatible providers and can redirect credentials |
| Strict OpenAI function schemas | Existing Wren schemas contain optional properties and are not all strict-mode compatible |
| Auth check during extension initialization | Makes normal startup require credentials and prevents credentialless discovery evidence |
| Treat missing credentials as a skipped/passing smoke | Violates the repository testing policy |
| Add `wren exec` now | Explicitly remains in #29 |

## Resolved decisions and limitations

No blocking product decision remains for implementation. The first version intentionally has:

- one non-streaming response per invocation;
- server-stored continuation through an opaque ID;
- no retry or rate-limit policy;
- no provider unload/reload;
- no provider concurrency guarantee;
- no pricing/cost calculation;
- text-only messages and tool results;
- function tools only; and
- one production authentication mechanism.

Those constraints are sufficient for #29's initial agent loop and should expand only from demonstrated needs.
