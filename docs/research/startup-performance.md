# Startup performance

Wren uses external tools to separate user-visible process measurement from function-level diagnosis:

- Hyperfine 1.20.0 measures the complete, uninstrumented release process.
- Samply 0.13.1 records sampled call stacks from an optimized build with debug symbols.

Neither tool is a Wren runtime dependency. Repository commands pin and invoke them without implementing timing, statistics, or profiling logic.

## Setup

Install both tools under ignored `target/perf-tools/`:

```console
cargo perf setup
```

Samply also relies on platform profiling support. On Windows, install the [Windows Performance Toolkit](https://learn.microsoft.com/windows-hardware/test/wpt/) so `xperf` is available and run profiling with administrator privileges. Linux may require permission to use performance events. On macOS, follow Samply's code-signing setup if requested.

The complete command interface is available through:

```console
cargo perf --help
```

## Startup measurement

```console
cargo perf startup
```

The command first builds Wren with Cargo's release profile. Hyperfine then launches the no-argument executable directly, with 20 warmups and at least 100 measured runs. Compilation and intermediate shell startup are outside the timed region.

The measurement includes operating-system process creation, dynamic-library loading, Wren initialization and execution, and process shutdown. It is a warm-process-start measurement: executable and library pages may already be cached.

Results are written to:

```text
target/perf/startup.json
target/perf/startup.md
```

### Initial baseline

The initial baseline measured the harness source at commit `a2612a6a3d3960f72ef6773d8797a161d9a0214d`.

| Field | Value |
| --- | --- |
| Date | 2026-07-24 |
| OS | Microsoft Windows 11 Home 10.0.26200, build 26200 |
| CPU | 12th Gen Intel Core i5-1235U, 10 cores / 12 logical processors |
| Rust | 1.97.1 (`8bab26f4f`), `x86_64-pc-windows-msvc` |
| Hyperfine | 1.20.0 |
| Runs | 20 warmups, 236 measured runs |
| Mean | 14.6 ms |
| Standard deviation | 4.0 ms |
| Median | 14.2 ms |
| Range | 7.2 ms to 28.1 ms |

This machine-local baseline is a reference, not a portable performance budget. Its variance demonstrates why regression decisions should compare baseline and candidate binaries in one run on the same machine.

## Regression comparison

Build the two revisions into separate target directories, then run:

```console
cargo perf compare <baseline-binary> <candidate-binary>
```

Use the release executables, adding `.exe` on Windows. Hyperfine measures both under the same conditions, treats the first as the reference, and writes:

```text
target/perf/startup-comparison.json
target/perf/startup-comparison.md
```

A slowdown is meaningful when it remains after repeating the comparison under quiet conditions and is larger than the reported uncertainty. A clear, unexplained slowdown blocks the change. Do not compare absolute results from different machines as a regression test.

## Function-level diagnosis

When Hyperfine identifies a meaningful slowdown, record a profile:

```console
cargo perf profile-startup
cargo perf view-profile
```

The first command builds Wren with the `profiling` Cargo profile, which retains release optimization and adds debug symbols. Samply samples 1,000 process launches at 10,000 Hz and saves `target/perf/startup-profile.json.gz`. The second command opens that artifact in the profiler UI for call-tree and flame-graph inspection.

Samply attributes sampled CPU activity; it does not replace the wall-clock benchmark. If elapsed startup regresses without a corresponding CPU call-path change, use platform tracing such as Windows Performance Analyzer, Linux `perf`, or Instruments to investigate I/O, scheduling, and other off-CPU causes.
