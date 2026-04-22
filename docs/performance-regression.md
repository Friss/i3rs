# Performance Regression Guardrails

This repository tracks performance in two layers:

- `crates/i3rs-core/benches/perf_benches.rs` provides Criterion numbers for the core hot paths.
- `crates/i3rs-app` can emit runtime percentile summaries behind the `perf_metrics` feature for end-to-end UI scenarios.

## CI Reporting

Pull requests run a report-only perf bench job in `.github/workflows/pr-verify.yml`.
The job does not enforce numeric thresholds yet; it uploads:

- `perf-bench-output.txt`
- `target/criterion/`

That keeps perf visible in CI while we gather more stable history across machines.

## Local Commands

Core benches:

```bash
cargo bench -p i3rs-core --bench perf_benches -- --output-format bencher
```

App runtime summaries:

```bash
cargo run -p i3rs-app --features perf_metrics
```

## Acceptance Targets

These come from the performance pass plan and are the thresholds we expect future work to preserve:

| Area | Target |
| --- | --- |
| Main-thread stall after file selection | under 16 ms for steady-state interactions |
| First visible plot after channel add | UI remains responsive while background decode runs |
| Graph pan/zoom p95 | at least 2x better than the pre-pass baseline on an 8-channel tiled view |
| Track hover latency | visibly smooth and below pre-pass baseline |
| Physical channel decode lifetime | decode at most once per loaded session unless the session changes |

## Current Baseline

The table below is the repo-tracked baseline captured after the shared-cache, background-work, and render-cache pass landed. Future changes should compare against these numbers and update the table intentionally when a new stable baseline is accepted.

| Benchmark | Current baseline |
| --- | --- |
| `LdFile::read_channel_data/VIR_LAP/Engine Speed` | 3,132 ns/iter (+/- 41) |
| `downsample_minmax/synthetic_1m_to_2k` | 437,853 ns/iter (+/- 1,359) |
| `evaluate_expression_with_aliases/synthetic` | 2,491,055 ns/iter (+/- 53,978) |
| `compute_fft_with_planner/synthetic_32k` | 238,436 ns/iter (+/- 2,122) |
| `find_nearest_sample/synthetic_200k` | 5,209 ns/iter (+/- 40) |

## Before/After Tracking

Use this template when capturing a new round of perf results for a change:

| Scenario | Before | After | Notes |
| --- | --- | --- | --- |
| Open `VIR_LAP.ld` |  |  |  |
| Open synthetic large session |  |  |  |
| Add several graph channels |  |  |  |
| Pan/zoom graph |  |  |  |
| Track map hover |  |  |  |
| Edit multi-input math channel |  |  |  |
