---
name: wren-testing
description: Plans and validates Wren unit tests, functional tests, authenticated provider coverage, and behavioral evaluations. Use when implementing or changing Wren features, tools, providers, prompts, schemas, permissions, tests, CI checks, or evaluation cases.
---

# Wren Testing

Read `docs/principles/testing.md` before changing production behavior or test infrastructure. Treat unit tests, functional tests, and behavioral evaluations as complementary evidence rather than substitutes.

## Classify the claim

State what the change must prove, then select the narrowest applicable layers:

- **Unit:** deterministic in-process logic and edge conditions.
- **Functional:** a representative feature scenario through the complete production boundary, with a binary deterministic verdict.
- **Authenticated functional:** a functional scenario whose real production boundary includes a provider/model.
- **Behavioral evaluation:** repeated real-agent attempts measuring reliability or efficiency.
- **Evaluator validation:** credentialless checks of evaluator mechanics only.

Every feature needs functional coverage. Add unit tests where they cheaply deepen branch and boundary coverage. Add behavioral evaluations only when the change can affect model-driven behavior or when a comparison is requested.

## Build functional coverage

Exercise the complete optimized Windows installation and real production process. Use isolated workspaces and home/configuration directories, but do not call them security sandboxes. Do not replace the boundary under test with mocks, fake providers, direct internal calls, manually assembled substitutes, or debug-only paths.

Keep each scenario focused on one production claim. Verify observable outputs, filesystem state, exit status, and errors deterministically. Cover representative success, boundary, and failure paths without reproducing every unit case end to end.

Run the repository entrypoint:

```console
cargo test --test functional
```

Always complete validation with the full optimized installation:

```console
cargo install-wren
```

If existing repository infrastructure cannot exercise the required installed production path, report and fix that gap rather than weakening the test claim.

## Handle credentials

Credentials are a capability requirement, not a test category.

- Use a real provider/model when that boundary is part of the functional claim.
- Fake providers are allowed for unit coverage but never establish authenticated functional coverage.
- Keep authenticated smoke scenarios narrow, inexpensive, and deterministically verified.
- Never pass secrets on command lines or write them to manifests, results, transcripts, patches, verifier artifacts, or logs.
- Give provider credentials only to the process that needs them; scrub them from verifier and unrelated harness environments.
- Do not run secrets against untrusted pull-request code.
- Never count missing credentials as a pass. An explicitly requested authenticated run must fail clearly if its requirements are unavailable.
- If applicable authenticated coverage cannot be run, stop before merge and request protected or human verification.

A single constrained real-provider run proves that the selected production path worked once. Repeated broader tasks measuring whether the agent chooses and uses that path are behavioral evaluations.

## Run behavioral evaluations

Use repository-owned evaluation commands once available; do not create ad hoc benchmark scripts or report a single attempt as behavioral evidence.

- Use the same versioned task, fixture, prompt, verifier, model/provider, reasoning, permissions, and budgets across compared harnesses.
- Run repeated paired or interleaved attempts.
- Keep infrastructure failures separate from task failures.
- Preserve machine-readable results and inspectable artifacts.
- Report the observed distribution, configuration, sample size, time, tokens/cost/tool calls when available, and limitations.
- Explain meaningful regressions before merging. Do not invent a fixed threshold before normal variance is measured.

Credentialless evaluator validation proves only runner mechanics. Do not cite it as evidence that Pi, Wren, a provider, or an agent behavior works.

## Apply the repository cadence

For each change:

1. During implementation, run relevant unit and credentialless functional tests.
2. Before completion, run all unit tests, credentialless functional tests, repository checks, and `cargo install-wren`.
3. For provider or orchestration changes, run applicable authenticated functional coverage in an approved environment.
4. For agent-visible behavior changes, run the relevant before/after behavioral evaluation and baseline comparison.
5. Record authenticated and behavioral evidence in the issue or pull request.

Load the separate `wren-performance` skill as well when the change may affect startup or another performance-sensitive runtime path.
