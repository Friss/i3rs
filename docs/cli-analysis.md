# i3rs CLI Analysis Reference

The analysis CLI is intended for agents and scripts that need deterministic,
machine-readable telemetry access without scraping the legacy text summary.

All JSON outputs include:

- `schema_version`: currently `i3rs-cli.analysis.v1`
- `command`: the command that produced the response
- `file`: input file metadata
- `warnings`: structured warnings when a command has recoverable issues

## Discovery Commands

```bash
i3rs summary session.ld --format json
i3rs channels session.ld --filter speed --format json
i3rs laps session.ld --source auto --format json
```

`summary` reports session/event metadata, duration, channel count, lap count,
sample-rate counts, and `.ldx` sidecar metadata when present.

`channels` reports channel index, name, unit, sample rate, sample count, data
type, decimal places, duration, and enum labels.

`laps` reports lap number, display name, start time, end time, and duration.
`--source auto` uses the same `.ld` lap detection as the app and includes `.ldx`
metadata. `--source ldx` uses sidecar marker pairs when available and falls back
to `.ld` detection with a structured warning when unavailable.

## Series Extraction

```bash
i3rs extract session.ld \
  --channel "Engine Speed" --channel "Throttle Position" \
  --lap "Lap 1" --x lap-percent --resample points:200 --format json
```

Selectors:

- `--time START:END`: absolute session seconds
- `--lap NAME|N`: one lap
- `--laps all` or `--laps A,B`: multiple lap segments

Axes:

- `time`: absolute session time
- `lap-time`: seconds from segment start
- `lap-percent`: 0 to 100 over the selected segment

Resampling:

- `native`: each channel returns native samples within the segment
- `HZ`: fixed-rate shared sample times, for example `20`
- `points:N`: N evenly spaced points, useful for lap-aligned analysis

Enum/state channels always emit native samples during extraction, even when a
fixed-rate or `points:N` resample is requested. This preserves short state
changes such as warnings that could be missed by point sampling.

CSV and NDJSON output use long rows:

```text
segment,lap_number,channel,x,absolute_time,value
```

## Statistics And Comparison

```bash
i3rs stats session.ld --channel "Engine Speed" --group lap \
  --metrics min,max,mean,stddev,p01,p50,p95,rms,integral --format json

i3rs compare-laps session.ld --channel "Engine Speed" \
  --laps "Lap 1,Lap 2" --reference "Lap 1" \
  --x lap-percent --resample points:200 --format json
```

Stats ignore non-finite values and always include `sample_count`,
`finite_count`, and `missing_count`.

Lap comparison normalizes each selected lap onto a shared axis before computing
`delta_mean`, `max_abs_delta`, `rmse`, and `area_delta`.

## Other Analysis Commands

```bash
i3rs histogram session.ld --channel "Engine Speed" --lap "Lap 1" --bins 50 --format json
i3rs math session.ld --expr 'WheelSlip=("Wheel Speed RL" - "GPS Speed") / "GPS Speed" * 100' --format json
```

`histogram` returns finite-value bin counts. `math` evaluates existing
`i3rs-core` expressions and reports derived-channel stats.

## Repeatable Run Specs

`run` accepts a JSON file with a top-level `file` and `operations` array:

```json
{
  "file": "test_data/VIR_LAP.ld",
  "operations": [
    { "type": "summary" },
    { "type": "channels", "filter": "speed" },
    { "type": "laps" },
    {
      "type": "stats",
      "channels": ["Engine Speed"],
      "group": "lap",
      "metrics": "min,max,mean,stddev"
    }
  ]
}
```

Run it with:

```bash
i3rs run analysis.json --format json
```

`run` supports `summary`, `channels`, `laps`, `extract`, `stats`,
`compare-laps`, `histogram`, and `math` operations. Operation fields mirror the
matching CLI flags using JSON names such as `channels`, `lap`, `laps`,
`resample`, `x`, `metrics`, and `expressions`.
