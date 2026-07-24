---
name: wren-performance
description: Measures and diagnoses Wren performance with Hyperfine and Samply. Use before and after changes that may affect startup, extension loading, command execution, or other performance-sensitive paths, and when investigating performance regressions.
---

# Wren Performance

Use Wren's repository-owned performance commands. Do not implement ad hoc timers, statistical comparisons, or profiler instrumentation.

## Prepare

Read `docs/research/startup-performance.md`, then verify the command interface:

```console
cargo perf --help
```

If the pinned tools are absent, install them into ignored `target/perf-tools/`:

```console
cargo perf setup
```

Samply requires platform profiling support. On Windows, verify that the Windows Performance Toolkit provides `xperf` and use an administrator session. Follow platform-specific guidance in the research document if profiling reports a permission or setup error.

## Measure startup

For a standalone measurement of the current release harness:

```console
cargo perf startup
```

Read `target/perf/startup.md` or `target/perf/startup.json`. This measures the real compiled harness process without compilation or an intermediate shell in the timed region.

## Compare a change

Build the baseline before editing into a separate target directory:

```console
cargo build --release --locked --package wren --target-dir target/perf-baseline
```

After editing, build the candidate separately:

```console
cargo build --release --locked --package wren --target-dir target/perf-candidate
```

Compare both executables in one Hyperfine invocation on the same machine:

```console
cargo perf compare target/perf-baseline/release/wren target/perf-candidate/release/wren
```

Use `wren.exe` on Windows. Read `target/perf/startup-comparison.md` or `target/perf/startup-comparison.json`.

If the difference is comparable to the reported uncertainty, reduce background activity and repeat. Do not claim an improvement or regression from a noisy result. Explain any clear slowdown before completing the change.

## Diagnose a regression

Record an optimized, symbol-enabled profile over repeated Wren launches:

```console
cargo perf profile-startup
```

Open the saved profile:

```console
cargo perf view-profile
```

Inspect the call tree and flame graph for new or enlarged Wren, Rust runtime, loader, allocator, and system-library call paths. Samply reports sampled CPU activity; if wall-clock startup regresses without a corresponding CPU-path change, investigate off-CPU causes with the platform's tracing tools.

## Report evidence

Record in the issue or pull request:

- Baseline and candidate commits.
- OS and CPU.
- Exact command.
- Mean, deviation, range, and relative comparison from Hyperfine.
- Whether the difference exceeds measurement uncertainty.
- For regressions, the dominant or newly introduced Samply call paths.

Do not commit generated files under `target/`. Do not merge an unexplained performance regression.
