---
name: wren-performance
description: Measures Wren performance with Hyperfine and diagnoses runtime paths with non-elevated Tracy instrumentation. Use before and after changes that may affect startup, extension loading, command execution, or other performance-sensitive paths, and when investigating regressions.
---

# Wren Performance

Use Wren's repository-owned commands. Do not implement ad hoc timers, statistical comparisons, trace parsers, or profiler instrumentation.

## Prepare

Read `docs/research/startup-performance.md`, then inspect the executable interface:

```console
cargo perf --help
```

Install missing pinned tools into ignored `target/perf-tools/`:

```console
cargo perf setup
```

All commands must run unattended from a standard non-elevated Windows session. Stop and report a defect if any command requests UAC, administrator access, or human interaction.

## Measure startup

```console
cargo perf startup
```

Read:

```text
target/perf/startup/results.md
target/perf/startup/results.json
```

This measures the real uninstrumented release process without compilation or an intermediate shell in the timed region.

## Compare a change

Build the baseline before editing into a separate target directory:

```console
cargo build --release --locked --package wren --target-dir target/perf-baseline
```

Build the candidate separately after editing:

```console
cargo build --release --locked --package wren --target-dir target/perf-candidate
```

Compare both executables in one Hyperfine invocation:

```console
cargo perf compare target/perf-baseline/release/wren.exe target/perf-candidate/release/wren.exe
```

Read:

```text
target/perf/startup-comparison/results.md
target/perf/startup-comparison/results.json
```

If a difference is comparable to the reported uncertainty, reduce background activity and repeat. Do not claim an improvement or regression from noisy results. Explain a repeatable slowdown before completing the change.

## Diagnose a regression

```console
cargo perf profile-startup
```

Read the official Tracy CSV directly:

```text
target/perf/startup-profile/zones.csv
```

Use zone names, source locations, durations, threads, nesting, and values to identify enlarged or newly introduced Wren paths. The command rejects empty captures and transactionally retains the corresponding trace at:

```text
target/perf/startup-profile/profile.tracy
```

A human can optionally open that trace with `cargo perf view-profile`. Agents do not need the GUI and must not wait for human inspection.

Tracy records instrumented execution rather than complete uninstrumented wall time. Use Hyperfine to decide whether a regression exists and Tracy to explain it.

## Add instrumentation

Profiling sites must follow the repository rules:

- Compile completely out of normal builds.
- Use static nested scopes for logical stacks.
- Prefer numeric values; use bounded text only at coarse boundaries.
- Never enable routine native call stacks, system tracing, sampling, or `flush-on-exit`.
- Scope a batch instead of each operation when individual work is expected below roughly 10 microseconds.
- Keep collector connection and drain waits outside measured scopes.

## Manage artifacts

```console
cargo perf clean
```

This removes generated results, intermediate comparison builds, and abandoned staging data while preserving pinned tool and Cargo profiling-build caches. Do not commit files under `target/`.

## Report evidence

Record in the issue or pull request:

- Baseline and candidate commits.
- Windows version, CPU, Rust, and Hyperfine versions.
- Exact comparison command.
- Mean, deviation, range, and relative comparison.
- Whether the difference exceeds measurement uncertainty.
- For regressions, the dominant or newly introduced Tracy zones.

Do not merge an unexplained performance regression.
