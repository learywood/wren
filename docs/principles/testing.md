# Testing and Evaluation

Wren uses unit tests, functional tests, and behavioral evaluations for different kinds of evidence. The layers are complementary: unit tests provide depth, functional tests provide production-path confidence, and evaluations measure real agent effectiveness. None substitutes for another.

## Definitions

### Unit tests

Unit tests exercise deterministic logic in-process and provide fast, localized failures.

- Use them for branch-heavy parsing, serialization, state transitions, boundary conditions, and error handling.
- Controlled collaborators are allowed, but a test using them makes no production integration claim.
- Do not require a unit test for every function or duplicate functional scenarios mechanically.
- Unit tests do not replace functional coverage.

A test's location does not determine its category. A Rust test under `tests/` that calls library internals is not functional merely because Cargo calls it an integration test.

### Functional tests

Functional tests prove that a production feature works through its real boundary.

- Every feature must have representative functional coverage.
- Exercise the complete release installation and supported native-Windows process boundary.
- Use the real bundled extensions, configuration layout, filesystem, and other production components involved in the claim.
- Do not use mocks, shims, alternate implementations, or shortcuts for the boundary being proven.
- Prepare an isolated workspace and home/configuration environment, but do not describe this reproducibility isolation as a security sandbox.
- Use deterministic assertions and report a binary verdict for each run.

The repository entrypoint remains:

```console
cargo test --test functional
```

### Behavioral evaluations

Behavioral evaluations measure how reliably and efficiently a real model-driven agent achieves an outcome.

- Express tasks as outcomes rather than direct tool invocations.
- Execute repeated attempts and report a distribution.
- Separate infrastructure failures from task failures.
- Preserve the task, configuration, transcript, artifacts, verifier output, and resulting patch.
- Use deterministic executable verification where possible.
- Compare configurations with paired or interleaved runs under pinned harness, model/provider, reasoning, permissions, task, and budget settings.

A functional test proves that a feature path worked in a specified scenario. An evaluation measures how often an agent chooses and uses available paths successfully. A single successful evaluation attempt does not establish reliability, and evaluation success does not replace functional coverage.

### Evaluator validation

Credentialless evaluator validation checks task manifests, workspace preparation, verifier execution, result serialization, and runner mechanics. It is ordinary deterministic repository validation and should run in CI, but provides no evidence about agent behavior.

## Credentials

Credentials do not define the boundary between functional tests and evaluations. The production feature under test determines whether credentials are required.

- A local tool path with no provider dependency must be functionally tested without credentials.
- Provider integration, model-backed agent execution, and model-driven tool orchestration must have functional coverage through the real provider/model path.
- A fake provider may support unit tests of orchestration states and error handling, but cannot establish the functional guarantee of a real provider path.
- Authenticated functional cases should be narrow, inexpensive smoke scenarios with deterministic verifiers. Do not assert incidental natural-language phrasing.
- Missing credentials must never be reported as a passing test. Default credentialless CI may omit explicitly authenticated cases, while an explicitly requested authenticated run must fail clearly when its required credentials are unavailable.
- Run authenticated cases locally or in protected CI; never expose credentials to untrusted pull-request code.
- Supply credentials through the normal production credential mechanism. Never store them in task manifests, command lines, result records, transcripts, patches, or verifier artifacts.
- Give credentials only to the process that needs them. Verifiers and unrelated harnesses should receive a scrubbed environment.

For example, a future `wren exec "prompt"` functional test may require credentials when its claim is that Wren can send a prompt to a real provider, process the response, invoke a tool, and return the result. A constrained prompt and deterministic verifier can prove that complete path worked once. Repeating broader prompt-driven tasks to estimate success rate is a behavioral evaluation instead.

Authenticated functional tests are required evidence for changes to the production paths they cover, but they are not part of ordinary untrusted pull-request CI. If required credentials are unavailable, stop before merging and request the required protected or human verification rather than substituting a fake path.

## When checks run

| Trigger | Required work |
|---|---|
| Fast development loop | Relevant unit tests and affected credentialless functional tests |
| Every push or pull request | Formatting, linting, build, all unit tests, all credentialless functional tests, evaluator validation, and complete release installation |
| Provider or orchestration change | Relevant authenticated functional tests before merge |
| Prompt, tool schema, permissions, or agent-behavior change | Paired repeated behavioral evaluations before merge, with results attached to the issue or pull request |
| Protected scheduled CI | Authenticated functional tests once those suites and credentials exist |
| Milestone or release | Complete applicable authenticated functional coverage and the compact behavioral corpus |
| Large benchmark | Manual execution for a concrete comparison, not routine CI |

Unit tests, credentialless functional tests, installation checks, and evaluator validation are hard CI gates. Authenticated functional tests are hard requirements for applicable changes but run only in an environment allowed to hold credentials. Behavioral evaluations are initially a review gate: present the evidence and explain meaningful regressions rather than inventing a numerical threshold before normal variance is known.

## Read tool example

The read tool illustrates the layers because it has complex local logic, a native extension boundary, and eventually model-driven use.

### Unit tests: does the read algorithm behave correctly?

Unit tests live beside `extensions/read/src/lib.rs` and call internal functions directly.

Representative cases include:

- Argument defaults produce `offset = 1` and `limit = 2000`.
- Empty paths and zero offsets or limits are rejected.
- CRLF is normalized correctly, including when `\r\n` crosses an internal buffer boundary.
- A UTF-8 character crossing the 50 KiB boundary is not split.
- Continuation notices fit inside the output limit.
- Exactly 2,000 lines do not claim truncation; 2,001 lines do.
- A long first line receives `[Line N truncated.]`.
- `notice_separator`, `decode_prefix`, and truncation-reason selection handle boundary cases.

Conceptually:

```rust
#[test]
fn requested_limit_reports_the_next_offset() {
    let collected = collect_fixture("one\ntwo\nthree\n", 2);
    let output = format_output(&collected, 1).unwrap();

    assert_eq!(
        output.text(),
        "one\ntwo\n\n[Showing lines 1-2. Use offset=3 to continue.]"
    );
}
```

These tests can cheaply exercise most branches and awkward byte boundaries. They guarantee that the tested Rust logic produces the expected result for controlled inputs. They do not prove that the extension DLL loads, Wren registers the tool, paths cross the process boundary correctly, the release contains the extension, or an agent can use it.

### Functional tests: does the shipped feature work?

A credentialless functional test installs the complete release into an isolated installation root and invokes the real process:

```console
wren tool read --args {"path":"sample.txt","offset":2,"limit":2}
```

A compact suite verifies:

1. **Happy path**
   - Install Wren and its bundled read extension.
   - Create `sample.txt` in an isolated workspace.
   - Invoke `wren.exe` with a relative path.
   - Assert exact stdout, empty stderr, and successful exit status.

2. **Path and environment integration**
   - Relative paths resolve against the process working directory.
   - Absolute Windows paths work.
   - A unique `WREN_HOME` is honored.

3. **Bounded output**
   - A file over 2,000 lines emits a correct continuation notice.
   - Output never exceeds 50 KiB.
   - A subsequent invocation using the advertised offset returns the next section.

4. **Production error behavior**
   - Missing file produces `not_found`.
   - A directory produces `not_regular_file`.
   - A locked Windows file produces `permission_denied`.
   - Invalid UTF-8 produces `invalid_utf8`.
   - Invalid arguments produce nonzero exit status and structured stderr.

5. **Packaging**
   - The read extension is present and automatically loaded from the release installation.

The existing `tests/functional.rs` covers many of these behaviors through a real Wren process. It currently uses a debug executable and manually constructs the extension installation, however, so it does not yet guarantee that `cargo install-wren` produced a working distribution.

Once Wren has agent orchestration, a protected authenticated functional smoke test can prove a further production claim: a real provider-driven Wren session receives the read definition, invokes the extension, and returns its result. A highly constrained prompt and deterministic verifier prove that the provider-to-agent-to-tool-to-extension path worked end to end once.

Functional coverage guarantees that specified scenarios work through the supported Windows production path. It does not prove every boundary input, consistent model tool selection, an effective tool description, parity with Pi, or behavior outside the tested scenario.

### Behavioral evaluation: can an agent use reading effectively?

An evaluation task expresses the outcome instead of invoking read directly.

Fixture:

- `records.txt` contains 2,500 plausible records.
- The required value occurs after line 2,000.
- `answer.txt` does not exist.

Prompt:

> Find the access code assigned to the `orchid` record in `records.txt`. Write only the code to `answer.txt`.

The deterministic verifier checks that `answer.txt` contains the expected code and no fixture files changed unexpectedly.

Pi and Wren receive the same workspace, prompt, model/provider, reasoning level, and comparable tool permissions. Each configuration receives repeated paired or interleaved attempts. Results preserve pass/fail, infrastructure failures, tool calls, continuation behavior, tokens and cost when available, wall time, transcript, and patch.

This supports a bounded conclusion such as:

> Under this task version, model, provider, tool configuration, and budget, Wren succeeded in 8/10 attempts and Pi in 9/10.

It does not prove universal correctness or future results. Provider changes, corpus selection, model variance, and sample size limit the conclusion.

## Development sequence

For a new capability or change:

1. Define the feature contract, including arguments, outputs, errors, and limits.
2. Add unit coverage for algorithmic branches and difficult boundaries while implementing.
3. Add or update functional coverage through the complete installed release. These tests are hard merge gates.
4. Run authenticated functional coverage when provider or tool orchestration is affected.
5. Run behavioral evaluations when descriptions, schemas, permissions, prompts, output shape, or other agent-visible behavior could change. Compare before and after and against the relevant baseline, and review unexplained regressions before merging.
