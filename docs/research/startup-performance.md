# Startup performance

Wren separates complete-process measurement from function-level diagnosis:

- Hyperfine 1.20.0 measures the uninstrumented release process.
- Tracy 0.13.1 records explicit scopes and values from an optimized profiling build.

Hyperfine remains authoritative for user-visible latency. Tracy is diagnostic instrumentation and is never enabled in a normal build.

## Setup

```console
cargo perf setup
```

The command installs pinned tools under ignored `target/perf-tools/`. It verifies the official Tracy Windows archive against SHA-256 `ee6db1a7e71a12deb5973a8dbfdf9f36d3635bec0e0b31b1cc74f28de7dac4c9` before publication. Setup and profiling run unattended in a standard non-elevated Windows session.

```console
cargo perf --help
```

## Startup measurement

```console
cargo perf startup
```

The command first builds Wren with Cargo's release profile. Hyperfine then launches the no-argument executable directly, with 20 warmups and at least 100 measured runs. Compilation and intermediate shell startup are outside the timed region.

Results are published transactionally to:

```text
target/perf/startup/results.json
target/perf/startup/results.md
```

The measurement includes Windows process creation, dynamic-library loading, Wren initialization, execution, and shutdown. It is a warm-process-start measurement because executable and library pages may already be cached.

### Recorded measurements

The first measurement, at commit `a2612a6a3d3960f72ef6773d8797a161d9a0214d`, reported 14.6 ms mean with 4.0 ms standard deviation over 236 runs on Windows 11 build 26200 and an Intel Core i5-1235U. The high variance makes this historical observation unsuitable as a budget.

A later 500-run control on the same machine measured an optimized empty Rust executable at `6.9 ms +/- 1.5 ms` and Wren at `7.4 ms +/- 1.7 ms`; their difference was within uncertainty. This demonstrates that most no-op latency is the Windows process floor. Regression decisions must compare baseline and candidate binaries together on the same machine rather than compare against either absolute number.

## Regression comparison

Build revisions into separate target directories, then run:

```console
cargo perf compare <baseline-binary> <candidate-binary>
```

Use `.exe` paths on Windows. Hyperfine measures both in one invocation and publishes:

```text
target/perf/startup-comparison/results.json
target/perf/startup-comparison/results.md
```

Repeat a comparison under quiet conditions when a difference is comparable to the reported uncertainty. A repeatable, unexplained slowdown blocks the change.

## Function-level diagnosis

```console
cargo perf profile-startup
```

This command:

1. Builds the optimized `profiling` Cargo profile with Wren's `profiling` feature.
2. Starts the official `tracy-capture` collector on localhost.
3. Requires Wren to observe the collector before entering the measured root scope.
4. Captures for one bounded second while Wren waits outside the root scope for collector shutdown.
5. Exports every zone through the official `tracy-csvexport` tool.
6. Verifies that the CSV contains the resolved `wren.run` zone.
7. Atomically publishes the completed output directory.

Retained outputs are:

```text
target/perf/startup-profile/profile.tracy
target/perf/startup-profile/zones.csv
```

Agents inspect `zones.csv`, whose established Tracy format contains zone name, source file and line, timestamp, duration, thread, and value. Humans can optionally open the same capture with:

```console
cargo perf view-profile
```

No ETW kernel logger, Windows Performance Toolkit, administrator access, UAC approval, custom trace parser, system sampling, or native call-stack collection is involved.

## Instrumentation rules

The optional `tracy-client` dependency has default features disabled. Only `enable`, `ondemand`, and `only-localhost` are enabled. `flush-on-exit` is prohibited because an unconnected process can wait indefinitely.

- Use static nested scopes to represent the logical call stack.
- Prefer numeric values for tags.
- Attach bounded text only to coarse scopes where it materially aids diagnosis.
- Do not request native call stacks at routine sites.
- Put one scope around a batch when individual operations are expected to take less than roughly 10 microseconds.
- End measured scopes before profile-only connection or drain waits.

Normal builds do not include `tracy-client`; profiling macros expand to no-op values and optimize away. The Windows overhead spike recorded on issue #5 found no measurable compiled-out cost, about 7-9 ns per disconnected on-demand site, and about 60-106 ns per connected static or tagged site. Native stack collection cost about 0.58-1.08 microseconds and remains opt-in only.

## Artifact lifecycle

- `target/perf-tools/` is a retained, ignored cache of checksum-verified tools.
- `.staging-*` directories contain incomplete runs and are deleted on normal failure.
- Download archives and capture intermediates are temporary and are deleted before publication.
- Named directories under `target/perf/` are the latest retained results.
- `target/perf-baseline/` and `target/perf-candidate/` are intermediate comparison builds.
- `target/profiling/` is Cargo's reusable optimized profiling-build cache.
- Publication swaps a complete staged directory into place; a failed run leaves the previous result intact.

Remove retained results and abandoned staging data deterministically with:

```console
cargo perf clean
```

The command removes retained results, intermediate comparison builds, and abandoned staging data. It preserves installed tools and Cargo's profiling-build cache so later runs remain offline except for Cargo builds.
