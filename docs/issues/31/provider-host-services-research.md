# Provider host-services research

> **Issue:** [#31 — Add provider extensions with OpenAI support](https://github.com/learywood/wren/issues/31)
>
> **Status:** Architecture research checkpoint. The earlier native provider-stream design is reopened; this document is not implementation approval.
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

## Unresolved architecture questions

The host-services direction is promising but not implementation-ready. The next design must resolve:

1. the versioned `HostServices` shape and how an extension receives and retains it;
2. whether to adopt `async-ffi` or use a smaller contract-specific future/poll wrapper under Wren's existing exact-fingerprint policy;
3. the owned HTTP response/body handle, allocator ownership, creator-side destruction, and DLL retention rules;
4. provider-stream polling, explicit cancellation versus drop, timeout terminal semantics, and wake behavior;
5. whether provider streams must be `Send`, and which executor configuration Wren actually needs;
6. bounded buffering and backpressure between Reqwest body chunks, provider parsing, and Wren event consumption;
7. transport error categories and safe bounded diagnostics without provider semantics in the host;
8. startup behavior and whether Tokio/Reqwest code can remain completely lazy on credentialless startup; and
9. interaction with transactional reload and safe generation unloading in issues #25 and #26.

## Next research task

Before another design-approval request:

1. Build a disposable Windows-only architecture spike, not production code, that loads a fixture DLL and proves a host-created async operation can be awaited and woken through the DLL boundary.
2. Compare `async-ffi` with a contract-specific opaque future/stream wrapper for safety, ownership clarity, allocations, panic behavior, and API evolution.
3. Extend the spike to a host-owned Reqwest request against a loopback SSE server; prove response-head delivery, byte streaming, cancellation, timeout, drop, and creator-side destruction.
4. Exercise controlled wire fixtures representative of OpenAI Responses, Anthropic Messages, OpenRouter comments/mid-stream errors, and OpenCode's mixed OpenAI/Anthropic routing. The host must remain unchanged across all fixtures.
5. Determine the minimal timeout model from observed provider needs: response-head, first-event, idle, and overall operation deadlines.
6. Verify that no host-owned task or waker can call unloaded extension code, and align the result with #25/#26 lifecycle plans.
7. Measure the real installed release startup before and after lazy runtime/HTTP integration using the repository performance workflow.
8. Return with the exact host-service and provider-stream contracts, rejected alternatives, compatibility evidence, and a revised acceptance plan for explicit approval.

Production implementation of issue #31 remains paused until that research is complete and approved.
