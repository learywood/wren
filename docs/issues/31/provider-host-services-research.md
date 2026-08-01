# Provider host-services research

> **Issue:** [#31 — Add provider extensions with OpenAI support](https://github.com/learywood/wren/issues/31)
>
> **Status:** The Windows host-services spike passed. Post-spike review simplified the recommended production direction to a fixed extension executor and shared network service; the exact [provider executor design proposal](provider-executor-design.md) requires explicit approval.
>
> **Session:** Pi session `019fae21-8e7a-7ae6-b4f9-1c499dc5fa0d`, 2026-07-29.

## Session checkpoint

Review of the original provider-extension design exposed a deeper problem: a provider DLL that directly uses an async HTTP library may carry a separately linked runtime and reactor that cannot automatically use the harness runtime polling its future. Giving every provider DLL its own runtime would complicate startup, cancellation, unloading, and connection management. A separate provider process was considered and rejected as a poor fit for Wren.

Pi does not solve this native boundary. At `c820aa26fe0907e053e881a957722693fc094c9c`, Pi loads JavaScript extensions into the same Node/Bun runtime, providers are ordinary in-runtime objects, and the built-in OpenAI implementation lazily imports its API module. Promises, network I/O, cancellation, and provider code therefore share one runtime without a binary boundary.

The direction preferred in this session is a single-process host-services model:

- Wren owns the async runtime, HTTP implementation, connection pools, TLS, proxy behavior, timers, and transport cancellation.
- Every extension can use the same provider-neutral host services.
- Provider extensions own endpoint selection, authentication-header construction, request mapping, SSE and JSON interpretation, replay state, errors, and normalized provider events.
- No Tokio, Reqwest, or provider-specific type crosses the DLL boundary.
- The host understands HTTP bytes, not OpenAI, Anthropic, models, tools, reasoning, or usage.

This is analogous to the facilities that the JavaScript runtime supplies implicitly to Pi extensions. It also makes transport setup a harness responsibility rather than repeated provider-extension work.

## Reference implementation findings

The synchronized reference checkouts were inspected without modifying them:

| Repository | Revision | Relevant finding |
|---|---|---|
| `earendil-works/pi` | `c820aa26fe0907e053e881a957722693fc094c9c` | Providers and extensions share one JavaScript runtime. OpenAI is built into `pi-ai`; extension registration passes in-runtime provider objects. |
| `can1357/oh-my-pi` | `639bac596d94` | Provider wire adapters share runtime `fetch`, normalized event streams, cancellation, and common SSE utilities. Provider identity is separate from wire API, allowing gateways to route different models through OpenAI Responses, OpenAI Chat Completions, or Anthropic Messages. |
| `openai/codex` | `f029bb795ccb` | The strongest Rust precedent. `codex-http-client` is the intended sole owner of direct Reqwest integration and exposes generic request, response, streaming byte, and transport types. `codex-api` owns provider request mapping and SSE parsing with `eventsource-stream`. |
| `0xPlaygrounds/rig` | `fb3b347d884b` | `HttpClientExt` abstracts unary and streaming HTTP; provider clients are generic over an injected HTTP backend, with Reqwest as the default. OpenAI, Anthropic, and OpenRouter adapters add their own URLs, headers, payloads, and stream interpretation. |
| `Dicklesworthstone/pi_agent_rust` | `9fcdb655cfb7` | A shared purpose-built HTTP/TLS client supplies bounded streaming bytes to multiple provider adapters. Providers own protocol translation and SSE interpretation. It demonstrates the value of centralized limits and connection setup, though it has no DLL boundary. |
| `vinhnx/VTCode` | `19ace7724a53` | Providers share common request, stream, and error types but retain Reqwest clients in provider configuration. It demonstrates cross-provider normalization but is a weaker transport separation reference. |
| `fortunto2/rust-code` | `e8245c0bf2fc` | Its OpenAI-compatible client directly owns Reqwest and reuses one adapter for OpenAI, OpenRouter, and Ollama. Useful only as evidence that gateways differ mainly in endpoint, headers, and payload compatibility. |
| `agentclientprotocol/agent-client-protocol` | `dc3bdddbfd95` | The unstable provider control-plane schema separates provider identity from protocol (`anthropic`, `openai`, `azure`, and others) and base URL. It does not implement provider transport. |

### Closest structural references

Codex already draws almost the same internal boundary proposed for Wren:

```text
codex-http-client
  owns Reqwest, proxy policy, connection pooling, redirects, generic byte streams

codex-client
  owns generic retries and attempt policy

codex-api
  owns OpenAI request types, endpoint mapping, SSE parsing, provider errors
```

Rig independently supports the same separation through an injectable `HttpClientExt` with unary and streaming calls. Neither project crosses a native DLL boundary, but both show that provider implementations do not need to own connection management.

`async-ffi` 0.5.1 is a relevant boundary library because it supplies FFI-compatible futures, task contexts, poll results, and waker forwarding. It does not supply an FFI stream abstraction, so Wren would still need a small owned `poll_next` wrapper for HTTP body chunks and provider events. `futures-core` supplies the semantic stream contract; `tokio`, `reqwest`, and `tokio-util` are suitable host-internal runtime, HTTP, and cancellation components. `eventsource-stream` is used by both Codex and Rig and is a candidate provider-side SSE parser once a Wren HTTP body handle is adapted to a normal byte stream.

`abi_stable` and `stabby` address independently built stable Rust ABIs. Wren currently requires lockstep source builds and an exact fingerprint, so adopting either would broaden this issue without solving transport ownership. They should not be added unless Wren changes that compatibility policy.

## Provider compatibility check

First-party documentation was checked on 2026-07-29 together with the reference implementations:

- [Anthropic streaming Messages](https://platform.claude.com/docs/en/build-with-claude/streaming)
- [OpenRouter API overview](https://openrouter.ai/docs/api_reference/overview)
- [OpenRouter streaming](https://openrouter.ai/docs/api_reference/streaming)
- [OpenCode Zen](https://opencode.ai/docs/zen/)
- [OpenCode Go](https://opencode.ai/docs/go/)
- the OpenAI sources already listed in `provider-extension-design.md`

The proposed raw HTTP host service is compatible with the required inference paths:

| Provider | Wire surfaces | Transport requirements | Result |
|---|---|---|---|
| OpenAI | `POST /v1/responses` with JSON and SSE | Bearer header, arbitrary JSON body, response status/headers, streamed bytes, cancellation, bounded error body | Compatible |
| Anthropic | `POST /v1/messages` with JSON and named SSE events | `x-api-key` or bearer auth, `anthropic-version`/beta headers, ping and error events, streamed bytes, cancellation | Compatible |
| OpenRouter | OpenAI-compatible Chat Completions and Responses; normalized SSE | Bearer and optional attribution headers, SSE comments, `[DONE]`, mid-stream error objects, generation response header | Compatible |
| OpenCode Zen | Per-model OpenAI Responses, OpenAI-compatible Chat Completions, Anthropic Messages, and additional future protocols | Arbitrary endpoint and headers, JSON POST, streamed bytes; model-specific wire adapter selected above transport | Compatible for the three listed HTTP/SSE wire APIs |
| OpenCode Go | Per-model OpenAI-compatible Chat Completions or Anthropic Messages | Same generic HTTP/SSE service; Go's Anthropic-compatible route may require `x-api-key` while other routes use bearer auth | Compatible |

OpenCode is important evidence against coupling provider identity to one wire protocol. Zen currently exposes models across OpenAI Responses, OpenAI-compatible Chat Completions, Anthropic Messages, and Google-compatible endpoints. Go currently mixes OpenAI-compatible and Anthropic-compatible endpoints. A future OpenCode extension must select a wire adapter by model metadata while reusing the same host HTTP service.

Compatibility here means the host service can carry the documented inference traffic. It does not pull dynamic model catalogs, OAuth/login UI, pricing, or credential management into issue #31. A general unary HTTP response is nevertheless useful for future `GET /models` discovery; restricting the service to only streaming POST would be an unnecessary dead end.

## Minimum host HTTP capability

The narrow transport surface supported by the research is:

- arbitrary standard HTTP method and absolute HTTP/HTTPS URL;
- ordered request headers with an explicit sensitive-value marker;
- an owned, bounded request body, initially sufficient for JSON;
- response status and ordered headers;
- an asynchronous response-body byte stream;
- a bounded collect operation for JSON responses and provider error bodies;
- cancellation that wakes pending work and drops active socket operations;
- caller-selected response-head, stream-idle, and overall deadlines rather than one inappropriate timeout for every phase;
- redirect handling that removes sensitive headers when the origin changes;
- shared connection pooling, TLS, DNS, and proxy policy inside Wren; and
- no request-body or sensitive-header logging.

SSE framing remains provider-side. This is necessary because provider behavior differs after framing: Anthropic has named events and ping/error events, OpenRouter adds comments and mid-stream errors, and OpenAI Responses has its own event taxonomy. Wren should not acquire those semantics.

Multipart uploads, WebSockets, bidirectional streaming, custom certificate APIs, provider SDK objects, and model-catalog policy are not required for issue #31. The host API can receive a new revision if a demonstrated provider later requires one of them.

## Windows native spike

A disposable spike was built under ignored `target/issue-31-host-services-spike/` at Wren commit `7fda67aaf935f69eed0a490ab6f2ec60ead7d5e4`. It was not added to Wren's workspace or production code.

Environment:

- Windows 11 Home build 26200;
- Intel Core i5-1235U;
- `rustc 1.97.1 (8bab26f4f 2026-07-14)` for `x86_64-pc-windows-msvc`;
- `async-ffi 0.5.1`, Tokio 1.53.1, Reqwest 0.13.4, and libloading 0.9.0; and
- a release `cdylib` loaded through the real Windows DLL boundary.

The spike had three crates:

```text
spike-host.exe
  Tokio runtime, Reqwest client, loopback HTTP/SSE server, DLL loader

provider_fixture.dll
  provider request construction, SSE parsing, event interpretation
  no Tokio, Reqwest, Hyper, TLS, socket, or reactor dependency

spike-abi
  repr(C) request/result types, async-ffi future, opaque body handle
```

The relevant experimental boundary was:

```text
HostServices { context, send_request }

send_request(HttpRequest)
  -> async-ffi FfiFuture<HeadResult>

HeadResult
  -> status + headers + HostBody

HostBody
  -> poll_next(handle, FfiContext)  // contract-specific direct poll
  -> next_future(handle)            // async-ffi comparison path
  -> cancel(handle)
  -> drop(handle)                    // creator-side destructor
```

Request values were copied synchronously before the host-created future was returned. Host-created response headers and chunks carried host destructor callbacks. The provider-created final result carried a provider-DLL destructor callback. A wrapper retained `Arc<Library>` until the provider future was dropped.

### Evidence

| Claim | Spike evidence |
|---|---|
| A host future can run through a provider DLL on Wren's runtime | The DLL awaited a host-created Reqwest future while its outer provider future was polled by the host Tokio executor. Delayed network chunks woke the task through both async-FFI boundaries. |
| Providers do not need a runtime or HTTP client | `cargo tree -p provider-fixture` contained only `async-ffi`, `serde_json`, and `spike-abi`; Tokio and Reqwest appeared only under `spike-host`. |
| Response heads and streaming bodies cross the ABI | The DLL checked status and response headers, then consumed deliberately fragmented chunked HTTP bodies through both body-poll variants. |
| Provider-neutral transport supports the target protocols | Controlled SSE fixtures covered OpenAI Responses, Anthropic named events and pings, OpenRouter comments and mid-stream errors, OpenAI-compatible Chat Completions, and OpenCode's mixed Chat/Anthropic routing. Provider parsing changed; the host did not. |
| Cancellation wakes pending work | Host-side cancellation woke a DLL future blocked in body polling, produced a cancellation terminal, dropped the unfinished Reqwest body, and caused the server to observe disconnect. |
| Timeouts require phases | Separate response-head and body-byte-idle timeouts both produced deterministic terminals. An overall timer was carried by the body state but its expiry path was not separately exercised. |
| Drop cancels unfinished work | Aborting the host task dropped the DLL future, which invoked the host future's creator-side destructor and disconnected a request waiting for response headers. |
| Panics need explicit policy | `async-ffi` caught a DLL future poll panic and re-raised it in the host task; Tokio contained it as a failed task. A custom body callback used `catch_unwind` to convert an injected panic into a typed terminal. Neither path silently crossed the C ABI. |
| A generation must remain loaded | Deleting the copied DLL failed while an in-flight provider future retained it. After aborting and dropping that future, Windows allowed immediate DLL deletion. |
| Creator-side allocation ownership works | Every body handle and host byte buffer was destroyed by host callbacks; every provider result was destroyed by a DLL callback. No allocator was used to free another module's allocation. |

The complete executable passed ten consecutive release runs. Every run covered twelve semantic cases—six protocols through each of two body-poll variants—plus cancellation, response-head timeout, idle timeout, pending-future drop, both panic paths, and unload retention. Representative counters were:

```json
{
  "semantic_cases": 12,
  "body_drops": 15,
  "per_chunk_ffi_futures": 24,
  "direct_poll_calls": 92,
  "send_future_drops": 18,
  "server_disconnects_observed": 5,
  "dll_generation_pinned_until_future_drop": "passed"
}
```

Expected panic hooks wrote diagnostics during the two contained panic cases; all spike processes still exited successfully.

### `async-ffi` versus direct polling

| Concern | `async-ffi` future per operation | Contract-specific direct `poll_next` |
|---|---|---|
| Waker correctness | Supplies an existing reviewed `FfiContext` and waker bridge | Should reuse `async-ffi::FfiContext`; independently recreating its waker machinery adds avoidable unsafe code |
| Allocation | `into_ffi` allocates once per wrapped future; the comparison made 24 extra per-chunk futures | No future allocation per body poll; returned chunks still require owned buffers |
| Panic behavior | Catches poll panic, crosses a `Panicked` marker, then re-panics in the consumer; drop or waker-vtable panic aborts | Every callback must catch panic itself and return a terminal; missing one would permit undefined behavior |
| Ownership | Built-in creator-side future destructor | Explicit opaque-handle destructor and one-in-flight-poll rule are required |
| Evolution | Has its own ABI version, which Wren must include in the extension fingerprint | Wren controls the small stream ABI and can revise it with the extension API |
| Best use | One future at a coarse async boundary | Repeated body and provider-event stream polling |

The spike initially suggested a hybrid async ABI. Subsequent review rejected that as unnecessary production complexity: Wren can preserve a responsive frontend and streaming without exposing any future, waker, or async stream through the DLL contract.

## Post-spike simplification

The current recommendation separates three execution contexts:

```text
Wren frontend/event loop
  submits provider jobs; receives events, completion, and cancellation

Fixed provider executor
  one lazy, reusable OS thread; invokes provider DLL synchronously

Shared network service
  one lazy Wren-owned Tokio/Reqwest runtime; owns every HTTP operation
```

This is not one thread per request. The provider executor is reused across calls. Issue #31 requires only one active provider turn at a time; if demonstrated future concurrency requires more, its internal executor can become a small bounded pool without changing the frontend, extension, event, cancellation, or HTTP contracts.

The extension-facing call remains synchronous and runs only on the provider executor:

```text
Provider::invoke(request, host_services, event_sink) -> ProviderResult
```

During that call:

1. The provider constructs its endpoint, headers, and JSON and calls blocking `host_services.http_start`.
2. The host copies the request and submits it to its network runtime.
3. The provider executor waits for the response head, then pulls bytes with blocking `body_next` calls.
4. A capacity-one host channel between Reqwest and `body_next` bounds buffering and applies backpressure.
5. The provider parses SSE and emits normalized events through a synchronous sink that only enqueues host-owned events; it never calls frontend code directly.
6. The frontend consumes those events independently and remains responsive.
7. Completion or cancellation returns from the synchronous DLL invocation before Wren releases that extension generation.

Cancellation belongs to the provider job, not the extension. Every host HTTP operation created during the job inherits its cancellation token. Cancelling the job aborts Reqwest, sends a cancellation terminal through the body channel, wakes the provider executor, and lets the DLL call return. Dropping an unfinished host body has the same transport-cancellation effect.

Host network tasks retain no DLL pointers, callbacks, futures, or wakers. All calls into the DLL occur on the provider executor, and all calls from the DLL into host services are synchronous. Safe generation unloading therefore reduces to retaining the generation for the invocation and releasing it only after the executor returns.

### Decisions simplified by this model

The production design does not need:

- `async-ffi`;
- cross-DLL futures, wakers, async streams, or `Send` requirements;
- one worker thread per request;
- a blocked frontend thread;
- an opaque asynchronous provider-event stream;
- provider-managed runtimes or HTTP clients; or
- a separate provider process.

The spike remains useful evidence for the underlying host-owned transport, streaming, cancellation, ownership, and Windows unload behavior. Its async ABI was an experiment, not a requirement to carry into production.

### Relationship to references

This preserves the transport/provider split demonstrated by Codex, Rig, Pi, and `pi_agent_rust`: shared infrastructure owns generic HTTP bytes, while providers own request and event semantics. The fixed executor is only Wren's native-DLL adapter. Those references do not need one because their provider implementations already share a language runtime or are statically linked.

Compared with a fully shared async runtime, the executor serializes provider invocations in issue #31. That is an intentional simplification for Wren's current sequential agent workload. The network service remains asynchronous and can multiplex operations; frontend responsiveness and streamed events do not depend on provider-call concurrency.

## Remaining exact design work

Before implementation, the approved design still needs to specify:

1. the versioned synchronous `HostServices`, host-body, provider invocation, event-sink, buffer, and creator-side destructor types;
2. the provider job's event, completion, overall-deadline, and request-scoped cancellation behavior;
3. bounded request, response-header, body-chunk, queued-event, final-response, and diagnostic sizes;
4. terminal ordering, event-sink backpressure, provider panic conversion, and cancellation while parsing or emitting;
5. lazy provider-executor and network-service startup and shutdown, plus generation retention during reload;
6. narrow redirect, retry, proxy, TLS, timeout, and credential-redaction policy without provider semantics in the host; and
7. controlled fixture, installed release, authenticated OpenAI, cancellation, unload/reload, startup-performance, and failure-path acceptance evidence.

The resulting exact contract and acceptance proposal is recorded in [Provider executor design proposal](provider-executor-design.md). Production implementation of issue #31 remains paused pending its approval.
