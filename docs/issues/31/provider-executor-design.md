# Provider executor design proposal

> **Issue:** [#31 — Add provider extensions and one-turn model execution](https://github.com/learywood/wren/issues/31)
>
> **Status:** Revised proposal. Issue #31 now ends at an installed one-turn `wren exec` path; production code must not change until the remaining exact command and JSONL contract is approved.

## Outcome

Wren will expose providers as native extension capabilities and consume them through an installed one-turn `wren exec` command, while keeping the frontend responsive, keeping all networking in one Wren-owned service, and keeping futures and wakers out of the DLL contract.

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

The initial host uses a bounded `crossbeam-channel` queue. The sink selects between sending the next event and job cancellation, so cancellation wakes a producer blocked by a full queue. `wren exec` drains the receiver while writing JSONL; a future interactive frontend can select or poll the same receiver without running provider code on its event thread. The provider executor never calls frontend code directly.

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

The host enforces start-before-content, matching start/delta/end kinds by content index, and exactly one terminal event. Multiple indexed items may be in progress when the provider protocol permits it. Safely retained partial events remain available before an error terminal; error values do not duplicate them.

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

The public provider call has an optional overall deadline. `wren exec --timeout <seconds>` accepts a positive integer number of seconds; omitting it applies no Wren overall deadline. Expiry follows the cancellation path but terminates as `timeout`, not `aborted`. Zero, fractions, and invalid values are rejected. The first revision has no provider-selectable response-head, byte-idle, or first-meaningful-event timer. Controlled Rust tests may use finer durations without exposing that precision in the CLI.

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

## One-turn command boundary

Issue #31 adds the real installed command:

```text
wren exec --provider <name> --model <id> --reasoning low [--timeout <seconds>]
```

The prompt is read from stdin, never argv. `--provider`, `--model`, and `--reasoning` are explicit; an empty value is rejected. The command sends no tool definitions and performs one provider invocation. It drains normalized events into a stable forward-compatible JSONL protocol on stdout, keeps diagnostics on stderr, and exits nonzero after an error terminal. An unexpected tool call is reported as unsupported in this no-tool command rather than executed.

The command is the production consumer of `ExtensionHost` and `ProviderJob`; #29 extends it with allowed tools and repeated turns rather than replacing it. There is no separate raw provider-invocation command.

## Shutdown ordering

Normal `ExtensionHost` shutdown:

1. stops accepting jobs;
2. cancels the active provider job, if any;
3. wakes a full event sink and every pending HTTP exchange;
4. waits for `Provider::invoke` to return;
5. destroys provider and extension-owned state while its DLL remains mapped;
6. stops and joins the network service; and
7. joins the extension executor.

Native extensions are trusted and must honor cancellation. Wren cannot safely kill a thread executing arbitrary DLL code; until #26 proves unloading, a non-returning extension blocks graceful shutdown and its generation remains mapped.

## Implementation boundary

Production work is limited to:

- the one-turn `wren exec` command and JSONL protocol;
- API revision 3 provider/request/event/host-service types;
- provider registration and dispatch;
- the fixed extension executor and lazy host network service;
- the bundled OpenAI Responses extension;
- installer integration; and
- issue #31 tests and evidence.

It does not add an agent/tool loop, tool orchestration, sessions, a model catalog, login UI, pricing, evaluator integration, or behavioral evaluation.

## Acceptance evidence

### Unit and controlled contract

- Command parsing, stdin handling, JSONL framing, request and event validation, sequence state machine, limits, usage normalization, and redaction.
- Provider registration, selection, empty/duplicate/changed metadata, and conflicts.
- An installed fixture DLL proving immediate job return, delayed streamed events through `wren.exe`, bounded backpressure, successful text/usage completion, provider and malformed errors, pre-cancellation, in-flight cancellation, explicit timeout, no default timeout, job drop, provider panic containment, and generation retention.
- Loopback HTTP proving response heads, repeated headers, split chunks, capacity-one backpressure, body drop, disconnect, and transport failures through the production network service.

### Controlled OpenAI

- Exact request path, headers, model, low reasoning, tools, full replay, encrypted reasoning, and correlated results.
- Ordered text/reasoning/tool-call SSE events, usage, unknown-event tolerance, malformed known events, HTTP/provider errors, missing terminal, timeout, and cancellation.
- Missing/empty key and exact sentinel-key redaction.

### Installed and authenticated

- `cargo install-wren` packages and auto-loads the ordinary OpenAI extension without credentials or network access.
- Installed credentialless functional tests invoke `wren.exe exec` with the fixture provider and cover CLI validation, provider selection, stdin, delayed JSONL streaming, cancellation, explicit timeout, omitted timeout, and errors.
- Credentialless test/build children explicitly remove `OPENAI_API_KEY`.
- An ignored local authenticated smoke invokes the installed `wren.exe exec` process, calls the real OpenAI endpoint with `gpt-5.6-sol` and low reasoning, verifies one fixed token, no tool calls, one success terminal, and positive consistent usage, and records no secret data.

This proves one installed production model call. Tool use and repeated agent turns remain in #29.

### Final gates

- formatting, linting, all unit and credentialless functional tests, and evaluator validation;
- complete optimized `cargo install-wren`;
- local authenticated OpenAI smoke;
- baseline/candidate startup comparison using `cargo perf compare`; and
- nonsecret authenticated and performance evidence attached to the issue and pull request.

No behavioral evaluation is required because issue #31 performs one explicitly requested model call without model-driven tool orchestration; #29 owns the behavioral task and baseline.

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
| Provider-only issue with a test host | Cannot demonstrate the complete installed functional path. Rejected in favor of one-turn `wren exec`. |

## Approval requested

The execution, provider, HTTP, cancellation, timeout, lifetime, and acceptance direction is agreed. Before production implementation, the stable JSONL event fields and exact no-tool command error behavior must be added here and explicitly approved.
