# Provider executor design proposal

> **Issue:** [#31 — Add provider extensions with OpenAI support](https://github.com/learywood/wren/issues/31)
>
> **Status:** Proposed after the Windows host-services spike and post-spike simplification. Production code must not change until this exact direction is approved.

## Outcome

Wren will expose providers as native extension capabilities while keeping the frontend responsive, keeping all networking in one Wren-owned service, and keeping futures and wakers out of the DLL contract.

The production path has three components:

```text
frontend / command coordinator
  submits work and drains bounded provider events

extension executor
  one fixed reusable thread; owns the extension registry and invokes DLL code synchronously

network service
  one lazy Wren-owned Tokio/Reqwest runtime; owns HTTP, TLS, pooling, and cancellation
```

There is no process per provider, runtime per extension, thread per request, or blocking provider call on the frontend thread.

## Execution model

`ExtensionHost` owns the extension registry on one named OS thread. The extension instances are created, invoked, and destroyed on that thread, so the native contract does not require `Extension`, `Provider`, or `Tool` to be `Send`. Existing synchronous tool commands submit a host command and wait for its result. A future frontend submits a provider command and immediately receives a `ProviderJob`.

A `ProviderJob` owns:

- a bounded host-owned receiver for normalized events;
- request-scoped cancellation; and
- the completion represented by exactly one terminal event.

The receiver supports blocking consumption for the current command/test hosts and asynchronous consumption by a future Tokio frontend. The provider executor blocks only when the bounded event queue is full; it never calls frontend code directly.

Issue #31 permits one active provider invocation at a time. This matches Wren's sequential agent turn and lets the same thread retain ordinary mutable extension semantics. If demonstrated future work requires concurrent sessions, Wren may shard extension instances or add owned call objects behind `ExtensionHost`; the provider, event, HTTP, and frontend job contracts do not change.

The network service starts only on the first HTTP request. It runs a Tokio current-thread runtime on one reusable background thread and creates one shared Reqwest client there. Tokio's own bounded blocking pool may be used by its DNS/TLS implementation; Wren does not create a worker per provider request.

## Native API revision 3

The build fingerprint increases from API revision 2 to 3. Tools remain source-compatible except for rebuilding against the new revision.

`Extension` adds indexed provider access parallel to tools:

```rust
pub trait Extension {
    fn initialize(&mut self) -> Result<ExtensionMetadata<'_>, ExtensionError>;
    fn tool(&mut self, index: usize) -> Option<&mut dyn Tool>;
    fn provider(&mut self, index: usize) -> Option<&mut dyn Provider>;
}
```

A provider is invoked synchronously only on the extension executor:

```rust
pub trait Provider {
    fn definition(&self) -> ProviderDefinition<'_>;

    fn invoke(
        &mut self,
        request: &ProviderRequest,
        context: &ProviderContext<'_>,
        events: &mut dyn ProviderEventSink,
    );
}
```

`invoke` must not panic. It emits borrowed events synchronously and must return promptly after the sink requests stop. It must emit exactly one `Done` or `Error` terminal. If it returns without one, emits after one, emits an invalid sequence, or panics, the host terminates the job as `malformed_response` or `provider` as applicable.

No provider-created allocation is returned for the host to destroy. Requests remain host-owned and borrowed for the call. Event strings, bytes, and JSON are borrowed only for the duration of `emit`; the host validates and copies accepted events before returning. Extension-owned state remains destroyed by the extension's existing creator-side destructor.

## Provider request

The host-owned `ProviderRequest` carries the minimum complete context required by #29:

- non-empty model ID;
- `low` reasoning effort;
- optional system/developer text;
- ordered user, assistant, and tool-result messages; and
- current function definitions.

Ordered assistant content supports:

- text with optional `commentary` or `final_answer` phase and opaque replay JSON;
- reasoning with optional model-visible summary and opaque replay JSON; and
- function calls with opaque call ID, non-empty name, and JSON-object arguments.

Tool results preserve call ID, tool name, text, and `is_error`. OpenAI receives complete replay history with `store: false`; server-stored continuation is not part of the common contract.

Images, audio, custom grammars, built-in provider tools, model catalogs, pricing, and credential management remain outside issue #31.

## Provider events

`ProviderEventSink::emit(ProviderEvent<'_>) -> EmitControl` uses the established Pi-like sequence:

```text
Start
TextStart / TextDelta / TextEnd
ReasoningStart / ReasoningDelta / ReasoningEnd
ToolCallStart / ToolCallDelta / ToolCallEnd
Done | Error
```

Content indexes preserve provider order. End events contain finalized content and replay metadata; `ToolCallEnd` contains parsed JSON-object arguments. `Done` contains stop reason and normalized usage. `Error` contains one stable category:

- `invalid_request`;
- `authentication`;
- `timeout`;
- `aborted`;
- `transport`;
- `provider`; or
- `malformed_response`.

`EmitControl` is `Continue` or `Stop`. The host returns `Stop` after cancellation, a sequence/limit violation, receiver closure, or a terminal event. The provider must then return without further events.

The host enforces start-before-content, matching start/delta/end kinds and indexes, one active item at a time, and exactly one terminal event. Safely retained partial events remain available before an error terminal; error values do not duplicate them.

The event queue holds at most 64 events. A single event may contain at most 1 MiB of text/JSON, and total finalized provider content may not exceed 16 MiB. Diagnostics are truncated to 8 KiB after redaction.

## Synchronous host HTTP service

`ProviderContext` exposes borrowed `HostServices` and cancellation state. The provider-facing operations are synchronous:

```rust
pub trait HostServices {
    fn http_start(&self, request: HttpRequest<'_>) -> HostHttpExchange;
    fn is_cancelled(&self) -> bool;
}

impl HostHttpExchange {
    fn wait_head(&mut self) -> HttpHeadResult<'_>;
    fn next_chunk(&mut self) -> HttpChunkResult<'_>;
}
```

`http_start` validates and copies all request data before returning. `wait_head` and `next_chunk` may block only the extension executor. Results borrow response headers, chunks, and diagnostic text from the host-owned exchange until its next mutable operation or destruction.

`HostHttpExchange` is an opaque native wrapper containing a host-created trait object and matching host destructor, analogous to `ExtensionInstance`. Dropping it calls host code and cancels unfinished work. The DLL never frees host request state, headers, chunks, diagnostics, channels, or network handles.

`HttpRequest` contains:

- arbitrary valid HTTP method and absolute HTTPS URL, with loopback HTTP allowed only by controlled tests;
- ordered headers with a sensitive-value marker; and
- one bounded in-memory request body.

The host supports repeated response headers and exposes status plus ordered response headers. It streams response bytes without interpreting SSE or provider JSON. Non-success HTTP statuses are ordinary response heads; the provider parses their bounded body and maps provider errors.

Initial host limits are:

| Value | Limit |
|---|---:|
| Method | 16 bytes |
| URL | 8 KiB |
| Request headers | 128 fields / 64 KiB total |
| Request body | 16 MiB |
| Response headers | 128 fields / 64 KiB total |
| Body chunk exposed to DLL | 64 KiB |
| Provider-collected error body | 1 MiB |

Reqwest chunks larger than 64 KiB are split. A capacity-one Tokio channel connects each network task to `next_chunk`, providing byte backpressure without blocking the runtime.

## Cancellation and deadlines

Every `ProviderJob` owns one cancellation token. Every exchange started through its `ProviderContext` receives a child token. Cancellation:

1. marks the event sink stopped;
2. aborts each active Reqwest operation;
3. closes/wakes pending head and body receives with `aborted`;
4. causes `HostServices::is_cancelled` to return true; and
5. requires the provider call to return before its extension generation can retire.

Dropping `ProviderJob` cancels it. Dropping an unfinished exchange cancels that exchange without affecting other jobs.

The public provider call has one overall deadline. Expiry follows the same path but terminates as `timeout`, not `aborted`. The first revision has no provider-selectable response-head, byte-idle, or first-meaningful-event timer. Reqwest has a 30-second connection timeout; the overall deadline governs response and stream duration.

## Transport and credential policy

The first revision intentionally has narrow policy:

- Reqwest uses Windows-native TLS and its normal system proxy discovery.
- Automatic redirects are disabled, preventing accidental forwarding of credentials.
- Automatic retries are disabled; provider retry semantics are outside issue #31.
- Host diagnostics never include request bodies or sensitive header values.
- The OpenAI extension reads `OPENAI_API_KEY` only inside `invoke`, marks `Authorization` sensitive, and never includes the exact key in an event or error.
- The production OpenAI endpoint is fixed to `https://api.openai.com/v1/responses`; loopback injection is compiled only for controlled extension tests.

The host classifies cancellation, timeout, invalid request construction, connection/TLS, and body transport failures. It does not classify OpenAI, Anthropic, OpenRouter, SSE, model, or tool semantics.

## Registry and lifetime

Provider registration mirrors tools:

1. validate contiguous indexed definitions during extension load;
2. reject empty and duplicate names;
3. reject cross-extension provider conflicts;
4. copy names into a provider-owner map; and
5. recheck the provider definition before each invocation.

Tools and providers use separate namespaces.

The extension executor retains the active loaded generation throughout `Provider::invoke`. Host network tasks contain only copied HTTP data, host channels, and cancellation tokens; they retain no DLL pointer or callback. The provider call must return and every provider-owned value must be destroyed before a generation can be considered drained.

Issue #25 may transactionally route new jobs to a new generation while an old job completes. Under issue #26, retired generations remain mapped until process exit unless its separate unload work proves all calls drained. Issue #31 does not implement hot reload or unloading.

## OpenAI extension

The ordinary bundled `openai` extension exposes provider `openai` and owns:

- endpoint and authorization headers;
- Responses request mapping;
- SSE framing and OpenAI event interpretation;
- text, reasoning, tool-call, replay, usage, and provider-error normalization; and
- enforcement of provider-specific accumulation limits.

Initialization reads no credential, creates no runtime/client, and performs no network access. Invocation uses `gpt-5.6-sol`, low reasoning, complete stateless replay, function definitions/results correlated by call ID, encrypted reasoning replay, and normalized Pi-compatible usage.

## Implementation boundary

Production work is limited to:

- API revision 3 provider/request/event/host-service types;
- provider registration and dispatch;
- the fixed extension executor and lazy host network service;
- the bundled OpenAI Responses extension;
- installer integration; and
- issue #31 tests and evidence.

It does not add `wren exec`, an agent loop, tool orchestration, sessions, a model catalog, login UI, pricing, or behavioral evaluation. There is no shipped raw provider-invocation command. Tests use the same production `ExtensionHost` library path that #29 will consume.

## Acceptance evidence

### Unit and controlled contract

- Request and event validation, sequence state machine, limits, usage normalization, and redaction.
- Provider registration, selection, empty/duplicate/changed metadata, and conflicts.
- A fixture DLL proving immediate job return, delayed streamed events, bounded backpressure, successful text/tool/usage completion, provider and malformed errors, pre-cancellation, in-flight cancellation, timeout, job drop, provider panic containment, and generation retention.
- Loopback HTTP proving response heads, repeated headers, split chunks, capacity-one backpressure, body drop, disconnect, and transport failures through the production network service.

### Controlled OpenAI

- Exact request path, headers, model, low reasoning, tools, full replay, encrypted reasoning, and correlated results.
- Ordered text/reasoning/tool-call SSE events, usage, unknown-event tolerance, malformed known events, HTTP/provider errors, missing terminal, timeout, and cancellation.
- Missing/empty key and exact sentinel-key redaction.

### Installed and authenticated

- `cargo install-wren` packages and auto-loads the ordinary OpenAI extension without credentials or network access.
- Credentialless test/build children explicitly remove `OPENAI_API_KEY`.
- An ignored local release integration smoke loads the installed DLL through production `ExtensionHost`, calls the real OpenAI endpoint with `gpt-5.6-sol` and low reasoning, verifies one fixed token, no tool calls, one success terminal, and positive consistent usage, and records no secret data.

This release integration proves the provider boundary, not a complete `wren.exe` agent command; #29 retains that production-process claim.

### Final gates

- formatting, linting, all unit and credentialless functional tests, and evaluator validation;
- complete optimized `cargo install-wren`;
- local authenticated OpenAI smoke;
- baseline/candidate startup comparison using `cargo perf compare`; and
- nonsecret authenticated and performance evidence attached to the issue and pull request.

No behavioral evaluation is required because issue #31 adds no agent command or model-driven orchestration.

## Alternatives and tradeoffs

| Alternative | Tradeoff and decision |
|---|---|
| Cross-DLL futures/wakers | Maximizes async concurrency but adds unsafe wake, `Send`, allocation, and unload lifetime surfaces. The spike proved it possible; this design rejects it as unnecessary. |
| One worker thread per provider request | Keeps a synchronous DLL but consumes a thread per active call. Rejected in favor of one reusable executor. |
| Invoke on the frontend thread | Smallest implementation but blocks input/rendering during network waits. Rejected. |
| Provider-owned Tokio/Reqwest | Avoids host callbacks but duplicates runtime, pooling, cancellation, startup, and unload work per DLL. Rejected. |
| Owned `Send` provider-call object | Could permit a worker pool, but adds another creator-owned native object and cross-thread DLL requirement before concurrent calls are demonstrated. Deferred. |
| Separate provider process | Provides isolation but violates the required extension model and adds IPC. Rejected. |
| Unary/nonstreaming OpenAI call | Smaller but removes incremental output and would change the provider/frontend contract when streaming is added. Rejected. |

## Approval requested

Approval authorizes this exact direction for production implementation, including the single serialized extension executor, synchronous native provider call with bounded event sink, lazy shared network service, overall-only public deadline, no redirects/retries, no raw provider CLI, and the stated limits and acceptance evidence. Requested changes should be made here before production code is edited.
