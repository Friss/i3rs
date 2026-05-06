//! CLI tool for parsing and analyzing MoTeC .ld files using i3rs-core.

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use i3rs_core::{
    ChannelData, ChannelMeta, Lap, LdFile, LdxFile, analysis::*, detect_laps, evaluate_expression,
    find_ldx_for_ld, is_state_channel, parse_expression, referenced_channels,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

const SCHEMA_VERSION: &str = "i3rs-cli.analysis.v1";
const SKILL_FILENAME: &str = "SKILL.md";
const I3RS_CLI_SKILL: &str = r#"---
name: i3rs
description: Use when programmatically inspecting and analyzing MoTeC .ld telemetry logs with i3rs JSON/CSV commands.
---

# i3rs CLI

Use `i3rs` when you need machine-readable analysis of a MoTeC `.ld` log file.

## Preferred Agent Workflow

1. Discover session shape: `i3rs summary session.ld --format json`.
2. Discover channel names: `i3rs channels session.ld --filter speed --format json`.
3. Discover lap windows: `i3rs laps session.ld --format json`.
4. Extract behavior over time instead of relying only on min/max:
   `i3rs extract session.ld --channel "Engine Speed" --lap "Lap 1" --x lap-percent --resample points:200 --format json`.
5. Compare laps:
   `i3rs compare-laps session.ld --channel "Engine Speed" --laps "Lap 1,In Lap" --reference "Lap 1" --x lap-percent --resample points:200 --format json`.

## Commands

- `i3rs <file.ld>` keeps the legacy text summary.
- `summary`, `channels`, `laps`, `extract`, `stats`, `compare-laps`, `histogram`, `math`, and `run` expose structured analysis commands.
- Use `--format json` for agent consumption and `--format csv` for tabular exports where supported.
- `install-skill [workspace-dir] [--force]` writes this helper skill to `.agents/skills/i3rs/SKILL.md` and `.claude/skills/i3rs/SKILL.md`.
"#;

#[derive(Parser)]
#[command(name = "i3rs", version, about = "MoTeC .ld telemetry analysis CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse a log and print metadata, channel counts, laps, and sample-rate summary.
    Summary(SummaryArgs),
    /// List channels and channel metadata.
    Channels(ChannelsArgs),
    /// List detected or sidecar lap windows.
    Laps(LapsArgs),
    /// Extract raw or resampled channel series.
    Extract(ExtractArgs),
    /// Compute session or per-lap channel statistics.
    Stats(StatsArgs),
    /// Compare lap-aligned channel behavior.
    #[command(name = "compare-laps")]
    CompareLaps(CompareLapsArgs),
    /// Compute histogram bins for a channel.
    Histogram(HistogramArgs),
    /// Evaluate a math expression as a derived channel.
    Math(MathArgs),
    /// Run a repeatable JSON analysis spec.
    Run(RunArgs),
    /// Write SKILL.md helpers for agents that use this CLI.
    #[command(name = "install-skill")]
    InstallSkill(InstallSkillArgs),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum TextJsonFormat {
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum DataFormat {
    Json,
    Csv,
    Ndjson,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum JsonCsvFormat {
    Json,
    Csv,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum LapSourceArg {
    Auto,
    Ld,
    Ldx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum XAxisArg {
    Time,
    #[serde(rename = "lap-time")]
    #[value(name = "lap-time")]
    LapTime,
    #[serde(rename = "lap-percent")]
    #[value(name = "lap-percent")]
    LapPercent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum StatsGroupArg {
    Session,
    Lap,
}

#[derive(Parser)]
struct SummaryArgs {
    file: PathBuf,
    #[arg(long, value_enum, default_value_t = TextJsonFormat::Text)]
    format: TextJsonFormat,
}

#[derive(Parser)]
struct ChannelsArgs {
    file: PathBuf,
    #[arg(long)]
    filter: Option<String>,
    #[arg(long, value_enum, default_value_t = TextJsonFormat::Text)]
    format: TextJsonFormat,
}

#[derive(Parser)]
struct LapsArgs {
    file: PathBuf,
    #[arg(long, value_enum, default_value_t = LapSourceArg::Auto)]
    source: LapSourceArg,
    #[arg(long, value_enum, default_value_t = TextJsonFormat::Text)]
    format: TextJsonFormat,
}

#[derive(Parser)]
struct ExtractArgs {
    file: PathBuf,
    #[arg(long = "channel", required = true)]
    channels: Vec<String>,
    #[arg(long)]
    time: Option<String>,
    #[arg(long)]
    lap: Option<String>,
    #[arg(long)]
    laps: Option<String>,
    #[arg(long = "x", value_enum, default_value_t = XAxisArg::Time)]
    x_axis: XAxisArg,
    #[arg(long, default_value = "native")]
    resample: String,
    #[arg(long, value_enum, default_value_t = DataFormat::Json)]
    format: DataFormat,
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Parser)]
struct StatsArgs {
    file: PathBuf,
    #[arg(long = "channel", required = true)]
    channels: Vec<String>,
    #[arg(long, value_enum, default_value_t = StatsGroupArg::Session)]
    group: StatsGroupArg,
    #[arg(long, default_value = "min,max,mean,stddev,p01,p50,p95,rms,integral")]
    metrics: String,
    #[arg(long, value_enum, default_value_t = JsonCsvFormat::Json)]
    format: JsonCsvFormat,
}

#[derive(Parser)]
struct CompareLapsArgs {
    file: PathBuf,
    #[arg(long = "channel", required = true)]
    channels: Vec<String>,
    #[arg(long)]
    laps: String,
    #[arg(long, default_value = "fastest")]
    reference: String,
    #[arg(long = "x", value_enum, default_value_t = XAxisArg::LapPercent)]
    x_axis: XAxisArg,
    #[arg(long, default_value = "points:200")]
    resample: String,
    #[arg(long, value_enum, default_value_t = JsonCsvFormat::Json)]
    format: JsonCsvFormat,
}

#[derive(Parser)]
struct HistogramArgs {
    file: PathBuf,
    #[arg(long)]
    channel: String,
    #[arg(long)]
    lap: Option<String>,
    #[arg(long, default_value_t = 50)]
    bins: usize,
    #[arg(long, value_enum, default_value_t = JsonCsvFormat::Json)]
    format: JsonCsvFormat,
}

#[derive(Parser)]
struct MathArgs {
    file: PathBuf,
    #[arg(long = "expr", required = true)]
    expressions: Vec<String>,
    #[arg(long)]
    lap: Option<String>,
    #[arg(long, value_enum, default_value_t = JsonCsvFormat::Json)]
    format: JsonCsvFormat,
}

#[derive(Parser)]
struct RunArgs {
    spec: PathBuf,
    #[arg(long, value_enum, default_value_t = TextJsonFormat::Json)]
    format: TextJsonFormat,
}

#[derive(Deserialize)]
struct AnalysisRunSpec {
    file: String,
    operations: Vec<RunOperation>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum RunOperation {
    Summary,
    Channels {
        filter: Option<String>,
    },
    Laps,
    Stats {
        channels: Vec<String>,
        #[serde(default = "default_stats_group")]
        group: StatsGroupArg,
        #[serde(default = "default_metrics_spec")]
        metrics: String,
    },
    Extract {
        channels: Vec<String>,
        time: Option<String>,
        lap: Option<String>,
        laps: Option<String>,
        #[serde(default = "default_extract_axis", rename = "x")]
        x_axis: XAxisArg,
        #[serde(default = "default_native_resample")]
        resample: String,
    },
    CompareLaps {
        channels: Vec<String>,
        laps: LapSelectorList,
        #[serde(default = "default_reference")]
        reference: String,
        #[serde(default = "default_compare_axis", rename = "x")]
        x_axis: XAxisArg,
        #[serde(default = "default_compare_resample")]
        resample: String,
    },
    Histogram {
        channel: String,
        lap: Option<String>,
        #[serde(default = "default_histogram_bins")]
        bins: usize,
    },
    Math {
        expr: Option<String>,
        expressions: Option<Vec<String>>,
        lap: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LapSelectorList {
    CommaSeparated(String),
    List(Vec<String>),
}

impl LapSelectorList {
    fn selectors(&self) -> Vec<String> {
        match self {
            Self::CommaSeparated(value) => parse_lap_selectors(value),
            Self::List(values) => values.clone(),
        }
    }
}

fn default_stats_group() -> StatsGroupArg {
    StatsGroupArg::Session
}

fn default_metrics_spec() -> String {
    "min,max,mean,stddev,p01,p50,p95,rms,integral".into()
}

fn default_extract_axis() -> XAxisArg {
    XAxisArg::Time
}

fn default_compare_axis() -> XAxisArg {
    XAxisArg::LapPercent
}

fn default_native_resample() -> String {
    "native".into()
}

fn default_compare_resample() -> String {
    "points:200".into()
}

fn default_reference() -> String {
    "fastest".into()
}

fn default_histogram_bins() -> usize {
    50
}

#[derive(Parser)]
struct InstallSkillArgs {
    target: Option<PathBuf>,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Clone, Serialize)]
struct WarningMessage {
    code: String,
    message: String,
}

#[derive(Debug, Clone)]
struct Segment {
    name: String,
    lap_number: Option<u32>,
    start_time: f64,
    end_time: f64,
}

#[derive(Debug, Clone, Copy)]
enum ResampleMode {
    Native,
    Hz(u16),
    Points(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Metric {
    Min,
    Max,
    Mean,
    Stddev,
    P01,
    P50,
    P95,
    Rms,
    Integral,
}

struct ExtractedSample {
    x: f64,
    absolute_time: f64,
    value: Option<f64>,
}

struct CompareLapsJsonParams<'a> {
    lap_selectors: &'a [String],
    reference_selector: &'a str,
    axis: XAxisArg,
    resample: ResampleMode,
}

fn fmt_val(v: Option<f64>) -> String {
    match v {
        None => "            ".to_string(),
        Some(v) if v.abs() < 0.01 && v != 0.0 => format!("{:12.4e}", v),
        Some(v) => format!("{:12.4}", v),
    }
}

fn skill_output_paths(target: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let path = match target {
        Some(target) => target.to_path_buf(),
        None => env::current_dir().map_err(|e| format!("could not read current directory: {e}"))?,
    };

    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    {
        Ok(vec![path])
    } else {
        Ok(vec![
            path.join(".agents")
                .join("skills")
                .join("i3rs")
                .join(SKILL_FILENAME),
            path.join(".claude")
                .join("skills")
                .join("i3rs")
                .join(SKILL_FILENAME),
        ])
    }
}

fn install_skill(args: InstallSkillArgs) -> Result<Vec<PathBuf>, String> {
    let output_paths = skill_output_paths(args.target.as_deref())?;
    let existing_paths = output_paths
        .iter()
        .filter(|path| path.exists())
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    if !existing_paths.is_empty() && !args.force {
        return Err(format!(
            "{} already exists; pass --force to overwrite it",
            existing_paths.join(", ")
        ));
    }

    for output_path in &output_paths {
        if let Some(parent) = output_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }

        fs::write(output_path, I3RS_CLI_SKILL)
            .map_err(|e| format!("could not write {}: {e}", output_path.display()))?;
    }

    Ok(output_paths)
}

fn open_ld(path: &Path) -> Result<LdFile, String> {
    LdFile::open(path).map_err(|err| err.to_string())
}

fn warn(code: &str, message: impl Into<String>) -> WarningMessage {
    WarningMessage {
        code: code.to_string(),
        message: message.into(),
    }
}

fn warning_values(warnings: &[WarningMessage]) -> Vec<Value> {
    warnings
        .iter()
        .map(|warning| json!({ "code": warning.code, "message": warning.message }))
        .collect()
}

fn channel_json(channel: &ChannelMeta) -> Value {
    let enum_labels: BTreeMap<i64, String> = channel
        .enum_labels
        .iter()
        .map(|(value, label)| (*value, label.clone()))
        .collect();

    json!({
        "index": channel.index,
        "name": channel.name,
        "short_name": channel.short_name,
        "unit": channel.unit,
        "frequency_hz": channel.freq,
        "sample_count": channel.n_data,
        "data_type": channel.data_type.name(),
        "decimal_places": channel.dec_places,
        "duration_secs": channel.duration_secs(),
        "enum_labels": enum_labels,
    })
}

fn lap_json(lap: &Lap) -> Value {
    json!({
        "number": lap.number,
        "name": lap.name,
        "start_time": lap.start_time,
        "end_time": lap.end_time,
        "duration": lap.duration(),
    })
}

fn ldx_metadata_json(ldx: Option<&LdxFile>) -> Value {
    match ldx {
        Some(ldx) => json!({
            "present": true,
            "total_laps": ldx.total_laps,
            "fastest_time": ldx.fastest_time,
            "fastest_lap": ldx.fastest_lap,
            "marker_pair_count": ldx.laps.len(),
        }),
        None => json!({ "present": false }),
    }
}

fn frequency_counts(ld: &LdFile) -> BTreeMap<u16, usize> {
    let mut freq_counts: BTreeMap<u16, usize> = BTreeMap::new();
    for ch in &ld.channels {
        *freq_counts.entry(ch.freq).or_insert(0) += 1;
    }
    freq_counts
}

fn load_laps(
    ld_path: &Path,
    ld: &LdFile,
    source: LapSourceArg,
) -> (Vec<Lap>, Option<LdxFile>, Vec<WarningMessage>, &'static str) {
    let ldx = find_ldx_for_ld(ld_path);
    let mut warnings = Vec::new();
    let detected = || detect_laps(ld);

    match source {
        LapSourceArg::Auto | LapSourceArg::Ld => {
            if source == LapSourceArg::Auto && ldx.is_none() {
                warnings.push(warn(
                    "missing_ldx",
                    "no .ldx sidecar was found; using .ld lap detection",
                ));
            }
            (detected(), ldx, warnings, "ld")
        }
        LapSourceArg::Ldx => match &ldx {
            Some(ldx) if !ldx.laps.is_empty() => {
                let laps = ldx
                    .laps
                    .iter()
                    .map(|lap| Lap {
                        number: lap.number + 1,
                        name: format!("Lap {}", lap.number + 1),
                        start_time: lap.start_time,
                        end_time: lap.end_time,
                    })
                    .collect();
                (laps, ldx.clone().into(), warnings, "ldx")
            }
            Some(_) => {
                warnings.push(warn(
                    "ldx_missing_markers",
                    ".ldx sidecar has metadata but no lap marker pairs; using .ld lap detection",
                ));
                (detected(), ldx, warnings, "ld")
            }
            None => {
                warnings.push(warn(
                    "missing_ldx",
                    "requested .ldx laps, but no .ldx sidecar was found; using .ld lap detection",
                ));
                (detected(), None, warnings, "ld")
            }
        },
    }
}

fn summary_json(
    path: &Path,
    ld: &LdFile,
    laps: &[Lap],
    ldx: Option<&LdxFile>,
    warnings: &[WarningMessage],
) -> Value {
    json!({
        "schema_version": SCHEMA_VERSION,
        "command": "summary",
        "file": {
            "path": path,
            "size_bytes": ld.file_size(),
        },
        "session": {
            "date": ld.session.date,
            "time": ld.session.time,
            "driver": ld.session.driver,
            "vehicle_id": ld.session.vehicle_id,
            "venue": ld.session.venue,
            "comment": ld.session.short_comment,
            "device_type": ld.session.device_type,
            "device_serial": ld.session.device_serial,
            "device_version": ld.session.device_version,
            "num_channels_header": ld.session.num_channels_header,
        },
        "event": {
            "event_name": ld.event.event_name,
            "session": ld.event.session,
            "comment": ld.event.comment,
            "venue_detail": ld.event.venue_detail,
            "vehicle_id": ld.event.vehicle_id,
            "vehicle_weight": ld.event.vehicle_weight,
            "vehicle_type": ld.event.vehicle_type,
            "vehicle_comment": ld.event.vehicle_comment,
        },
        "duration_secs": ld.duration_secs(),
        "channel_count": ld.channels.len(),
        "lap_count": laps.len(),
        "sample_rates": frequency_counts(ld),
        "ldx": ldx_metadata_json(ldx),
        "warnings": warning_values(warnings),
    })
}

fn print_summary_text(path: &Path, ld: &LdFile) {
    let sep = "-".repeat(100);
    let s = &ld.session;
    let e = &ld.event;

    println!("\nParsing: {}", path.display());
    println!("Size   : {} bytes\n", ld.file_size());

    println!("{}", sep);
    println!("  MoTeC i2 Log File Summary");
    println!("{}", sep);
    println!("  Date/Time     : {}  {}", s.date, s.time);
    println!("  Driver        : {}", s.driver);
    println!("  Vehicle ID    : {}", s.vehicle_id);
    println!("  Venue         : {}", s.venue);
    println!("  Comment       : {}", s.short_comment);
    println!(
        "  Device        : {} (serial {}, v{})",
        s.device_type, s.device_serial, s.device_version
    );
    println!(
        "  Channels      : {} (header), {} (parsed)",
        s.num_channels_header,
        ld.channels.len()
    );

    if !e.event_name.is_empty() {
        println!("\n  Event         : {}", e.event_name);
    }
    if !e.session.is_empty() {
        println!("  Session       : {}", e.session);
    }
    if !e.venue_detail.is_empty() {
        println!("  Venue Detail  : {}", e.venue_detail);
    }
    if !e.vehicle_id.is_empty() {
        println!(
            "  Vehicle       : {} (type={}, weight={})",
            e.vehicle_id, e.vehicle_type, e.vehicle_weight
        );
    }

    let duration = ld.duration_secs();
    if duration > 0.0 {
        let mins = (duration / 60.0) as u32;
        let secs = duration - (mins as f64 * 60.0);
        println!(
            "\n  Est. Duration : {}m {:.1}s  ({:.1}s)",
            mins, secs, duration
        );
    }

    println!("\n{}", sep);
    println!(
        "  {:>3}  {:<45} {:<8} {:>5} {:>8} {:<9} {:>12} {:>12} {:>12}",
        "#", "Channel Name", "Unit", "Hz", "Samples", "Type", "Min", "Max", "Mean"
    );
    println!("{}", sep);

    for ch in &ld.channels {
        let stats = ld
            .read_channel_data(ch)
            .map(|data| compute_stats(&data, None))
            .unwrap_or_else(|| compute_stats(&[], None));

        println!(
            "  {:3}  {:<45} {:<8} {:5} {:8} {:<9} {} {} {}",
            ch.index,
            ch.name,
            ch.unit,
            ch.freq,
            ch.n_data,
            ch.data_type.name(),
            fmt_val(stats.min),
            fmt_val(stats.max),
            fmt_val(stats.mean)
        );
    }

    println!("{}", sep);
    println!("  Total channels: {}", ld.channels.len());

    let parts: Vec<String> = frequency_counts(ld)
        .iter()
        .map(|(f, c)| format!("{} ch @ {} Hz", c, f))
        .collect();
    println!("  By sample rate: {}", parts.join(", "));
    println!("{}", sep);
}

fn print_channels_text(ld: &LdFile, filter: Option<&str>) {
    let normalized_filter = filter.map(|filter| filter.to_ascii_lowercase());
    println!(
        "{:>3}  {:<45} {:<8} {:>5} {:>8} {:<9} {:>6}",
        "#", "Channel Name", "Unit", "Hz", "Samples", "Type", "Dec"
    );
    for channel in filtered_channels(ld, normalized_filter.as_deref()) {
        println!(
            "{:3}  {:<45} {:<8} {:5} {:8} {:<9} {:6}",
            channel.index,
            channel.name,
            channel.unit,
            channel.freq,
            channel.n_data,
            channel.data_type.name(),
            channel.dec_places
        );
    }
}

fn filtered_channels<'a>(ld: &'a LdFile, filter: Option<&str>) -> Vec<&'a ChannelMeta> {
    ld.channels
        .iter()
        .filter(|channel| {
            filter.is_none_or(|filter| {
                channel.name.to_ascii_lowercase().contains(filter)
                    || channel.unit.to_ascii_lowercase().contains(filter)
            })
        })
        .collect()
}

fn print_laps_text(laps: &[Lap], source_used: &str) {
    println!("Lap source: {source_used}");
    println!(
        "{:>4}  {:<12} {:>12} {:>12} {:>12}",
        "#", "Name", "Start", "End", "Duration"
    );
    for lap in laps {
        println!(
            "{:4}  {:<12} {:12.6} {:12.6} {:12.6}",
            lap.number,
            lap.name,
            lap.start_time,
            lap.end_time,
            lap.duration()
        );
    }
}

fn resolve_channel<'a>(ld: &'a LdFile, name: &str) -> Result<&'a ChannelMeta, String> {
    ld.find_channel_by_name(name)
        .ok_or_else(|| format!("channel not found: {name}"))
}

fn resolve_channels<'a>(ld: &'a LdFile, names: &[String]) -> Result<Vec<&'a ChannelMeta>, String> {
    names.iter().map(|name| resolve_channel(ld, name)).collect()
}

fn parse_time_range(input: &str) -> Result<(f64, f64), String> {
    let Some((start, end)) = input.split_once(':') else {
        return Err("time range must be START:END seconds".into());
    };
    let start: f64 = start
        .parse()
        .map_err(|_| format!("invalid time range start: {start}"))?;
    let end: f64 = end
        .parse()
        .map_err(|_| format!("invalid time range end: {end}"))?;
    if !start.is_finite() || !end.is_finite() || end <= start {
        return Err("time range must be finite and END must be greater than START".into());
    }
    Ok((start, end))
}

fn parse_resample(input: &str) -> Result<ResampleMode, String> {
    let normalized = input.trim().to_ascii_lowercase();
    if normalized == "native" {
        return Ok(ResampleMode::Native);
    }
    if let Some(points) = normalized.strip_prefix("points:") {
        let points = points
            .parse::<usize>()
            .map_err(|_| format!("invalid points resample value: {input}"))?;
        if points == 0 {
            return Err("points resample value must be greater than zero".into());
        }
        return Ok(ResampleMode::Points(points));
    }
    let hz = normalized.strip_suffix("hz").unwrap_or(&normalized);
    let hz = hz
        .parse::<u16>()
        .map_err(|_| format!("invalid resample mode: {input}"))?;
    if hz == 0 {
        return Err("Hz resample value must be greater than zero".into());
    }
    Ok(ResampleMode::Hz(hz))
}

fn parse_lap_selectors(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn select_segments(
    ld: &LdFile,
    laps: &[Lap],
    time: Option<&str>,
    lap: Option<&str>,
    laps_arg: Option<&str>,
) -> Result<Vec<Segment>, String> {
    let requested_modes = [time.is_some(), lap.is_some(), laps_arg.is_some()]
        .into_iter()
        .filter(|selected| *selected)
        .count();
    if requested_modes > 1 {
        return Err("choose only one of --time, --lap, or --laps".into());
    }

    if let Some(range) = time {
        let (start_time, end_time) = parse_time_range(range)?;
        return Ok(vec![Segment {
            name: "time".into(),
            lap_number: None,
            start_time,
            end_time,
        }]);
    }

    if let Some(selector) = lap {
        let (_, lap) =
            find_lap(laps, selector).ok_or_else(|| format!("lap not found: {selector}"))?;
        return Ok(vec![Segment {
            name: lap.name.clone(),
            lap_number: Some(lap.number),
            start_time: lap.start_time,
            end_time: lap.end_time,
        }]);
    }

    if let Some(laps_arg) = laps_arg {
        if laps_arg.eq_ignore_ascii_case("all") {
            return Ok(laps
                .iter()
                .map(|lap| Segment {
                    name: lap.name.clone(),
                    lap_number: Some(lap.number),
                    start_time: lap.start_time,
                    end_time: lap.end_time,
                })
                .collect());
        }

        return parse_lap_selectors(laps_arg)
            .into_iter()
            .map(|selector| {
                let (_, lap) = find_lap(laps, &selector)
                    .ok_or_else(|| format!("lap not found: {selector}"))?;
                Ok(Segment {
                    name: lap.name.clone(),
                    lap_number: Some(lap.number),
                    start_time: lap.start_time,
                    end_time: lap.end_time,
                })
            })
            .collect();
    }

    Ok(vec![Segment {
        name: "session".into(),
        lap_number: None,
        start_time: 0.0,
        end_time: ld.duration_secs(),
    }])
}

fn x_value(axis: XAxisArg, segment: &Segment, absolute_time: f64) -> f64 {
    match axis {
        XAxisArg::Time => absolute_time,
        XAxisArg::LapTime => absolute_time - segment.start_time,
        XAxisArg::LapPercent => {
            let duration = segment.end_time - segment.start_time;
            if duration > 0.0 {
                (absolute_time - segment.start_time) / duration * 100.0
            } else {
                0.0
            }
        }
    }
}

fn extract_channel_samples(
    ld: &LdFile,
    channel: &ChannelMeta,
    segment: &Segment,
    axis: XAxisArg,
    resample: ResampleMode,
) -> Result<(ResampleMode, Vec<ExtractedSample>), String> {
    let effective_resample = if should_emit_native_samples(channel, resample) {
        ResampleMode::Native
    } else {
        resample
    };

    let samples = if matches!(effective_resample, ResampleMode::Native) {
        let window = sample_window_for_time(
            channel.freq,
            channel.n_data as usize,
            segment.start_time,
            segment.end_time,
        );
        ld.read_channel_range(channel, window.start_sample, window.end_sample)
            .ok_or_else(|| format!("failed to decode channel {}", channel.name))?
            .into_iter()
            .enumerate()
            .map(|(idx, value)| {
                let absolute_time = (window.start_sample + idx) as f64 / channel.freq as f64;
                ExtractedSample {
                    x: x_value(axis, segment, absolute_time),
                    absolute_time,
                    value: finite_value(value),
                }
            })
            .collect()
    } else {
        let times = match resample {
            ResampleMode::Native => Vec::new(),
            ResampleMode::Hz(hz) => fixed_hz_times(segment.start_time, segment.end_time, hz),
            ResampleMode::Points(points) => {
                evenly_spaced_times(segment.start_time, segment.end_time, points)
            }
        };
        let data = ld
            .read_channel_data(channel)
            .ok_or_else(|| format!("failed to decode channel {}", channel.name))?;
        times
            .iter()
            .copied()
            .zip(resample_at_times(&data, channel.freq, &times))
            .map(|(absolute_time, value)| ExtractedSample {
                x: x_value(axis, segment, absolute_time),
                absolute_time,
                value: value.and_then(finite_value),
            })
            .collect()
    };

    Ok((effective_resample, samples))
}

fn extract_json(
    path: &Path,
    ld: &LdFile,
    channels: &[&ChannelMeta],
    segments: &[Segment],
    axis: XAxisArg,
    resample: ResampleMode,
    warnings: &[WarningMessage],
) -> Result<Value, String> {
    let mut segment_values = Vec::new();
    for segment in segments {
        let mut channel_values = Vec::new();
        for channel in channels {
            let (effective_resample, data) =
                extract_channel_samples(ld, channel, segment, axis, resample)?;
            let data = data
                .into_iter()
                .map(|sample| {
                    json!({
                        "x": sample.x,
                        "absolute_time": sample.absolute_time,
                        "value": sample.value,
                    })
                })
                .collect::<Vec<_>>();
            channel_values.push(json!({
                "channel": channel_json(channel),
                "effective_resample": resample_name(effective_resample),
                "samples": data,
            }));
        }
        segment_values.push(json!({
            "name": segment.name,
            "lap_number": segment.lap_number,
            "start_time": segment.start_time,
            "end_time": segment.end_time,
            "duration": segment.end_time - segment.start_time,
            "channels": channel_values,
        }));
    }

    Ok(json!({
        "schema_version": SCHEMA_VERSION,
        "command": "extract",
        "file": { "path": path },
        "x_axis": axis_name(axis),
        "resample": resample_name(resample),
        "segments": segment_values,
        "warnings": warning_values(warnings),
    }))
}

fn finite_value(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn axis_name(axis: XAxisArg) -> &'static str {
    match axis {
        XAxisArg::Time => "time",
        XAxisArg::LapTime => "lap-time",
        XAxisArg::LapPercent => "lap-percent",
    }
}

fn resample_name(mode: ResampleMode) -> String {
    match mode {
        ResampleMode::Native => "native".into(),
        ResampleMode::Hz(hz) => format!("{hz}Hz"),
        ResampleMode::Points(points) => format!("points:{points}"),
    }
}

fn channel_uses_discrete_values(channel: &ChannelMeta) -> bool {
    !channel.enum_labels.is_empty() || is_state_channel(&channel.name)
}

fn should_emit_native_samples(channel: &ChannelMeta, resample: ResampleMode) -> bool {
    matches!(resample, ResampleMode::Native) || channel_uses_discrete_values(channel)
}

fn discrete_resample_warnings(
    channels: &[&ChannelMeta],
    resample: ResampleMode,
) -> Vec<WarningMessage> {
    if matches!(resample, ResampleMode::Native) {
        return Vec::new();
    }

    channels
        .iter()
        .filter(|channel| channel_uses_discrete_values(channel))
        .map(|channel| {
            warn(
                "discrete_native_samples",
                format!(
                    "{} is an enum/state channel; emitting native samples instead of {} resampling",
                    channel.name,
                    resample_name(resample)
                ),
            )
        })
        .collect()
}

fn extract_csv_or_ndjson(
    ld: &LdFile,
    channels: &[&ChannelMeta],
    segments: &[Segment],
    axis: XAxisArg,
    resample: ResampleMode,
    format: DataFormat,
) -> Result<String, String> {
    let mut out = String::new();
    if format == DataFormat::Csv {
        out.push_str("segment,lap_number,channel,x,absolute_time,value\n");
    }

    for segment in segments {
        for channel in channels {
            let (_, samples) = extract_channel_samples(ld, channel, segment, axis, resample)?;
            for sample in samples {
                push_sample_row(&mut out, format, segment, channel, sample);
            }
        }
    }
    Ok(out)
}

fn push_sample_row(
    out: &mut String,
    format: DataFormat,
    segment: &Segment,
    channel: &ChannelMeta,
    sample: ExtractedSample,
) {
    match format {
        DataFormat::Csv => {
            let value = sample
                .value
                .map_or_else(String::new, |value| value.to_string());
            out.push_str(&format!(
                "{},{},{},{:.9},{:.9},{}\n",
                csv_escape(&segment.name),
                segment
                    .lap_number
                    .map_or_else(String::new, |number| number.to_string()),
                csv_escape(&channel.name),
                sample.x,
                sample.absolute_time,
                value
            ));
        }
        DataFormat::Ndjson => {
            let value = json!({
                "schema_version": SCHEMA_VERSION,
                "segment": segment.name,
                "lap_number": segment.lap_number,
                "channel": channel.name,
                "x": sample.x,
                "absolute_time": sample.absolute_time,
                "value": sample.value,
            });
            out.push_str(&value.to_string());
            out.push('\n');
        }
        DataFormat::Json => {}
    }
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn parse_metrics(input: &str) -> Result<Vec<Metric>, String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|metric| !metric.is_empty())
        .map(|metric| match metric.to_ascii_lowercase().as_str() {
            "min" => Ok(Metric::Min),
            "max" => Ok(Metric::Max),
            "mean" | "avg" => Ok(Metric::Mean),
            "stddev" | "std" => Ok(Metric::Stddev),
            "p01" => Ok(Metric::P01),
            "p50" | "median" => Ok(Metric::P50),
            "p95" => Ok(Metric::P95),
            "rms" => Ok(Metric::Rms),
            "integral" => Ok(Metric::Integral),
            _ => Err(format!("unknown metric: {metric}")),
        })
        .collect()
}

fn stats_metric_value(stats: &AnalysisStats, metric: Metric) -> Option<f64> {
    match metric {
        Metric::Min => stats.min,
        Metric::Max => stats.max,
        Metric::Mean => stats.mean,
        Metric::Stddev => stats.stddev,
        Metric::P01 => stats.p01,
        Metric::P50 => stats.p50,
        Metric::P95 => stats.p95,
        Metric::Rms => stats.rms,
        Metric::Integral => stats.integral,
    }
}

fn metric_name(metric: Metric) -> &'static str {
    match metric {
        Metric::Min => "min",
        Metric::Max => "max",
        Metric::Mean => "mean",
        Metric::Stddev => "stddev",
        Metric::P01 => "p01",
        Metric::P50 => "p50",
        Metric::P95 => "p95",
        Metric::Rms => "rms",
        Metric::Integral => "integral",
    }
}

fn stats_json_value(stats: &AnalysisStats, metrics: &[Metric]) -> Value {
    let mut metric_map = serde_json::Map::new();
    for metric in metrics {
        metric_map.insert(
            metric_name(*metric).into(),
            json!(stats_metric_value(stats, *metric)),
        );
    }
    json!({
        "sample_count": stats.sample_count,
        "finite_count": stats.finite_count,
        "missing_count": stats.missing_count,
        "metrics": metric_map,
    })
}

fn stats_json(
    path: &Path,
    ld: &LdFile,
    channels: &[&ChannelMeta],
    laps: &[Lap],
    group: StatsGroupArg,
    metrics: &[Metric],
) -> Result<Value, String> {
    let mut rows = Vec::new();
    for channel in channels {
        let data = ld
            .read_channel_data(channel)
            .ok_or_else(|| format!("failed to decode channel {}", channel.name))?;
        match group {
            StatsGroupArg::Session => {
                rows.push(json!({
                    "channel": channel_json(channel),
                    "group": "session",
                    "stats": stats_json_value(&compute_stats(&data, Some(channel.freq)), metrics),
                }));
            }
            StatsGroupArg::Lap => {
                for lap in laps {
                    let window = sample_window_for_time(
                        channel.freq,
                        data.len(),
                        lap.start_time,
                        lap.end_time,
                    );
                    let slice = if window.is_empty() {
                        &[][..]
                    } else {
                        &data[window.start_sample..window.end_sample]
                    };
                    rows.push(json!({
                        "channel": channel_json(channel),
                        "group": "lap",
                        "lap": lap_json(lap),
                        "stats": stats_json_value(&compute_stats(slice, Some(channel.freq)), metrics),
                    }));
                }
            }
        }
    }

    Ok(json!({
        "schema_version": SCHEMA_VERSION,
        "command": "stats",
        "file": { "path": path },
        "group": match group { StatsGroupArg::Session => "session", StatsGroupArg::Lap => "lap" },
        "results": rows,
        "warnings": [],
    }))
}

fn stats_csv(
    ld: &LdFile,
    channels: &[&ChannelMeta],
    laps: &[Lap],
    group: StatsGroupArg,
    metrics: &[Metric],
) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("channel,group,lap,sample_count,finite_count,missing_count");
    for metric in metrics {
        out.push(',');
        out.push_str(metric_name(*metric));
    }
    out.push('\n');

    for channel in channels {
        let data = ld
            .read_channel_data(channel)
            .ok_or_else(|| format!("failed to decode channel {}", channel.name))?;
        match group {
            StatsGroupArg::Session => {
                push_stats_csv_row(
                    &mut out,
                    channel,
                    "session",
                    "",
                    &compute_stats(&data, Some(channel.freq)),
                    metrics,
                );
            }
            StatsGroupArg::Lap => {
                for lap in laps {
                    let window = sample_window_for_time(
                        channel.freq,
                        data.len(),
                        lap.start_time,
                        lap.end_time,
                    );
                    let slice = if window.is_empty() {
                        &[][..]
                    } else {
                        &data[window.start_sample..window.end_sample]
                    };
                    push_stats_csv_row(
                        &mut out,
                        channel,
                        "lap",
                        &lap.name,
                        &compute_stats(slice, Some(channel.freq)),
                        metrics,
                    );
                }
            }
        }
    }
    Ok(out)
}

fn push_stats_csv_row(
    out: &mut String,
    channel: &ChannelMeta,
    group: &str,
    lap: &str,
    stats: &AnalysisStats,
    metrics: &[Metric],
) {
    out.push_str(&format!(
        "{},{},{},{},{},{}",
        csv_escape(&channel.name),
        group,
        csv_escape(lap),
        stats.sample_count,
        stats.finite_count,
        stats.missing_count
    ));
    for metric in metrics {
        out.push(',');
        if let Some(value) = stats_metric_value(stats, *metric) {
            out.push_str(&value.to_string());
        }
    }
    out.push('\n');
}

fn fastest_lap(laps: &[Lap]) -> Option<(usize, &Lap)> {
    laps.iter()
        .enumerate()
        .filter(|(_, lap)| lap.name.starts_with("Lap "))
        .min_by(|(_, a), (_, b)| a.duration().total_cmp(&b.duration()))
        .or_else(|| {
            laps.iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.duration().total_cmp(&b.duration()))
        })
}

fn comparison_times(
    reference: &Lap,
    comparison: &Lap,
    mode: ResampleMode,
) -> (Vec<f64>, Vec<f64>, f64) {
    let points = match mode {
        ResampleMode::Native => 200,
        ResampleMode::Points(points) => points,
        ResampleMode::Hz(hz) => ((reference.duration() * hz as f64).ceil() as usize).max(1),
    };
    let fractions: Vec<f64> = match points {
        0 => Vec::new(),
        1 => vec![0.0],
        _ => (0..points)
            .map(|idx| idx as f64 / (points - 1) as f64)
            .collect(),
    };
    let ref_times = fractions
        .iter()
        .map(|frac| reference.start_time + reference.duration() * frac)
        .collect();
    let cmp_times = fractions
        .iter()
        .map(|frac| comparison.start_time + comparison.duration() * frac)
        .collect();
    let x_step = if points > 1 {
        1.0 / (points - 1) as f64
    } else {
        1.0
    };
    (ref_times, cmp_times, x_step)
}

fn compare_laps_json(
    path: &Path,
    ld: &LdFile,
    channels: &[&ChannelMeta],
    laps: &[Lap],
    params: CompareLapsJsonParams<'_>,
) -> Result<Value, String> {
    let CompareLapsJsonParams {
        lap_selectors,
        reference_selector,
        axis,
        resample,
    } = params;

    let (reference_idx, reference_lap) = if reference_selector.eq_ignore_ascii_case("fastest") {
        fastest_lap(laps).ok_or_else(|| "no laps available for fastest reference".to_string())?
    } else {
        find_lap(laps, reference_selector)
            .ok_or_else(|| format!("reference lap not found: {reference_selector}"))?
    };

    let selected_laps: Vec<(usize, &Lap)> = lap_selectors
        .iter()
        .map(|selector| {
            find_lap(laps, selector).ok_or_else(|| format!("lap not found: {selector}"))
        })
        .collect::<Result<_, _>>()?;

    let mut results = Vec::new();
    for channel in channels {
        let data = ld
            .read_channel_data(channel)
            .ok_or_else(|| format!("failed to decode channel {}", channel.name))?;
        for (comparison_idx, comparison_lap) in &selected_laps {
            if *comparison_idx == reference_idx {
                continue;
            }
            let (ref_times, cmp_times, x_step) =
                comparison_times(reference_lap, comparison_lap, resample);
            let reference_values = resample_at_times(&data, channel.freq, &ref_times);
            let comparison_values = resample_at_times(&data, channel.freq, &cmp_times);
            let metrics = compare_aligned(&reference_values, &comparison_values, x_step);
            let samples: Vec<Value> = ref_times
                .iter()
                .zip(cmp_times.iter())
                .zip(reference_values.iter().zip(comparison_values.iter()))
                .map(|((ref_time, cmp_time), (ref_value, cmp_value))| {
                    let fraction = if reference_lap.duration() > 0.0 {
                        (*ref_time - reference_lap.start_time) / reference_lap.duration()
                    } else {
                        0.0
                    };
                    let x = match axis {
                        XAxisArg::Time | XAxisArg::LapTime => reference_lap.duration() * fraction,
                        XAxisArg::LapPercent => fraction * 100.0,
                    };
                    json!({
                        "x": x,
                        "reference_time": ref_time,
                        "comparison_time": cmp_time,
                        "reference_value": ref_value,
                        "comparison_value": cmp_value,
                        "delta": match (ref_value, cmp_value) {
                            (Some(a), Some(b)) if a.is_finite() && b.is_finite() => Some(b - a),
                            _ => None,
                        },
                    })
                })
                .collect();

            results.push(json!({
                "channel": channel_json(channel),
                "reference_lap": lap_json(reference_lap),
                "comparison_lap": lap_json(comparison_lap),
                "metrics": metrics,
                "samples": samples,
            }));
        }
    }

    Ok(json!({
        "schema_version": SCHEMA_VERSION,
        "command": "compare-laps",
        "file": { "path": path },
        "x_axis": axis_name(axis),
        "resample": resample_name(resample),
        "results": results,
        "warnings": [],
    }))
}

fn compare_laps_csv(
    ld: &LdFile,
    channels: &[&ChannelMeta],
    laps: &[Lap],
    lap_selectors: &[String],
    reference_selector: &str,
    resample: ResampleMode,
) -> Result<String, String> {
    let (_, reference_lap) = if reference_selector.eq_ignore_ascii_case("fastest") {
        fastest_lap(laps).ok_or_else(|| "no laps available for fastest reference".to_string())?
    } else {
        find_lap(laps, reference_selector)
            .ok_or_else(|| format!("reference lap not found: {reference_selector}"))?
    };
    let selected_laps: Vec<&Lap> = lap_selectors
        .iter()
        .map(|selector| {
            find_lap(laps, selector)
                .map(|(_, lap)| lap)
                .ok_or_else(|| format!("lap not found: {selector}"))
        })
        .collect::<Result<_, _>>()?;

    let mut out = String::from(
        "channel,reference_lap,comparison_lap,point_count,finite_pair_count,delta_mean,max_abs_delta,rmse,area_delta\n",
    );
    for channel in channels {
        let data = ld
            .read_channel_data(channel)
            .ok_or_else(|| format!("failed to decode channel {}", channel.name))?;
        for comparison_lap in &selected_laps {
            if comparison_lap.name == reference_lap.name {
                continue;
            }
            let (ref_times, cmp_times, x_step) =
                comparison_times(reference_lap, comparison_lap, resample);
            let reference_values = resample_at_times(&data, channel.freq, &ref_times);
            let comparison_values = resample_at_times(&data, channel.freq, &cmp_times);
            let metrics = compare_aligned(&reference_values, &comparison_values, x_step);
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{},{}\n",
                csv_escape(&channel.name),
                csv_escape(&reference_lap.name),
                csv_escape(&comparison_lap.name),
                metrics.point_count,
                metrics.finite_pair_count,
                option_csv(metrics.delta_mean),
                option_csv(metrics.max_abs_delta),
                option_csv(metrics.rmse),
                option_csv(metrics.area_delta)
            ));
        }
    }
    Ok(out)
}

fn option_csv(value: Option<f64>) -> String {
    value.map_or_else(String::new, |value| value.to_string())
}

fn histogram_json(
    path: &Path,
    ld: &LdFile,
    channel: &ChannelMeta,
    segment: &Segment,
    bins: usize,
) -> Result<Value, String> {
    let data = ld
        .read_channel_data(channel)
        .ok_or_else(|| format!("failed to decode channel {}", channel.name))?;
    let window = sample_window_for_time(
        channel.freq,
        data.len(),
        segment.start_time,
        segment.end_time,
    );
    let slice = if window.is_empty() {
        &[][..]
    } else {
        &data[window.start_sample..window.end_sample]
    };

    Ok(json!({
        "schema_version": SCHEMA_VERSION,
        "command": "histogram",
        "file": { "path": path },
        "channel": channel_json(channel),
        "segment": {
            "name": segment.name,
            "lap_number": segment.lap_number,
            "start_time": segment.start_time,
            "end_time": segment.end_time,
        },
        "bins": histogram_bins(slice, bins),
        "warnings": [],
    }))
}

fn histogram_csv(
    ld: &LdFile,
    channel: &ChannelMeta,
    segment: &Segment,
    bins: usize,
) -> Result<String, String> {
    let value = histogram_json(Path::new(""), ld, channel, segment, bins)?;
    let mut out = String::from("segment,channel,lower,upper,count\n");
    for bin in value["bins"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "{},{},{},{},{}\n",
            csv_escape(&segment.name),
            csv_escape(&channel.name),
            bin["lower"].as_f64().unwrap_or_default(),
            bin["upper"].as_f64().unwrap_or_default(),
            bin["count"].as_u64().unwrap_or_default()
        ));
    }
    Ok(out)
}

fn parse_math_expr(input: &str) -> Result<(String, String), String> {
    let Some((name, expression)) = input.split_once('=') else {
        return Err("math expressions must be NAME=EXPR".into());
    };
    let name = name.trim();
    let expression = expression.trim();
    if name.is_empty() || expression.is_empty() {
        return Err("math expressions must have a non-empty NAME and EXPR".into());
    }
    Ok((name.to_string(), expression.to_string()))
}

fn evaluate_math_channels(
    ld: &LdFile,
    expressions: &[String],
    lap: Option<&Lap>,
) -> Result<Vec<Value>, String> {
    let mut results = Vec::new();
    for input in expressions {
        let (name, expression) = parse_math_expr(input)?;
        let expr = parse_expression(&expression).map_err(|err| err.to_string())?;
        let mut channel_data = HashMap::new();
        for reference in referenced_channels(&expr) {
            let channel = resolve_channel(ld, &reference)?;
            let samples = ld
                .read_channel_data(channel)
                .ok_or_else(|| format!("failed to decode channel {}", channel.name))?;
            channel_data.insert(
                channel.name.clone(),
                ChannelData {
                    samples,
                    freq: channel.freq,
                },
            );
        }
        let (samples, freq) = evaluate_expression(&expression, &channel_data)?;
        let segment = lap.map(|lap| Segment {
            name: lap.name.clone(),
            lap_number: Some(lap.number),
            start_time: lap.start_time,
            end_time: lap.end_time,
        });
        let slice = if let Some(segment) = &segment {
            let window =
                sample_window_for_time(freq, samples.len(), segment.start_time, segment.end_time);
            if window.is_empty() {
                &[][..]
            } else {
                &samples[window.start_sample..window.end_sample]
            }
        } else {
            &samples[..]
        };
        results.push(json!({
            "name": name,
            "expression": expression,
            "frequency_hz": freq,
            "sample_count": samples.len(),
            "lap": lap.map(lap_json),
            "stats": compute_stats(slice, Some(freq)),
        }));
    }
    Ok(results)
}

fn math_csv(ld: &LdFile, expressions: &[String], lap: Option<&Lap>) -> Result<String, String> {
    let rows = evaluate_math_channels(ld, expressions, lap)?;
    let mut out = String::from(
        "name,expression,frequency_hz,sample_count,finite_count,min,max,mean,stddev\n",
    );
    for row in rows {
        let stats = &row["stats"];
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            csv_escape(row["name"].as_str().unwrap_or_default()),
            csv_escape(row["expression"].as_str().unwrap_or_default()),
            row["frequency_hz"].as_u64().unwrap_or_default(),
            row["sample_count"].as_u64().unwrap_or_default(),
            stats["finite_count"].as_u64().unwrap_or_default(),
            json_option_csv(&stats["min"]),
            json_option_csv(&stats["max"]),
            json_option_csv(&stats["mean"]),
            json_option_csv(&stats["stddev"]),
        ));
    }
    Ok(out)
}

fn json_option_csv(value: &Value) -> String {
    value
        .as_f64()
        .map_or_else(String::new, |value| value.to_string())
}

fn write_output(output: Option<&Path>, content: &str) -> Result<(), String> {
    match output {
        Some(path) => fs::write(path, content)
            .map_err(|err| format!("could not write {}: {err}", path.display())),
        None => {
            print!("{content}");
            Ok(())
        }
    }
}

fn print_json(value: &Value) -> Result<(), String> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer_pretty(&mut lock, value).map_err(|err| err.to_string())?;
    writeln!(lock).map_err(|err| err.to_string())
}

fn handle_summary(args: SummaryArgs) -> Result<(), String> {
    let ld = open_ld(&args.file)?;
    let (laps, ldx, warnings, _) = load_laps(&args.file, &ld, LapSourceArg::Auto);
    match args.format {
        TextJsonFormat::Text => print_summary_text(&args.file, &ld),
        TextJsonFormat::Json => print_json(&summary_json(
            &args.file,
            &ld,
            &laps,
            ldx.as_ref(),
            &warnings,
        ))?,
    }
    Ok(())
}

fn handle_channels(args: ChannelsArgs) -> Result<(), String> {
    let ld = open_ld(&args.file)?;
    match args.format {
        TextJsonFormat::Text => print_channels_text(&ld, args.filter.as_deref()),
        TextJsonFormat::Json => {
            let filter = args
                .filter
                .as_ref()
                .map(|filter| filter.to_ascii_lowercase());
            print_json(&json!({
                "schema_version": SCHEMA_VERSION,
                "command": "channels",
                "file": { "path": args.file },
                "filter": args.filter,
                "channels": filtered_channels(&ld, filter.as_deref())
                    .into_iter()
                    .map(channel_json)
                    .collect::<Vec<_>>(),
                "warnings": [],
            }))?;
        }
    }
    Ok(())
}

fn handle_laps(args: LapsArgs) -> Result<(), String> {
    let ld = open_ld(&args.file)?;
    let (laps, ldx, warnings, source_used) = load_laps(&args.file, &ld, args.source);
    match args.format {
        TextJsonFormat::Text => print_laps_text(&laps, source_used),
        TextJsonFormat::Json => print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "command": "laps",
            "file": { "path": args.file },
            "source_requested": format!("{:?}", args.source).to_ascii_lowercase(),
            "source_used": source_used,
            "ldx": ldx_metadata_json(ldx.as_ref()),
            "laps": laps.iter().map(lap_json).collect::<Vec<_>>(),
            "warnings": warning_values(&warnings),
        }))?,
    }
    Ok(())
}

fn handle_extract(args: ExtractArgs) -> Result<(), String> {
    let ld = open_ld(&args.file)?;
    let channels = resolve_channels(&ld, &args.channels)?;
    let (laps, _, mut warnings, _) = load_laps(&args.file, &ld, LapSourceArg::Auto);
    let segments = select_segments(
        &ld,
        &laps,
        args.time.as_deref(),
        args.lap.as_deref(),
        args.laps.as_deref(),
    )?;
    if segments.is_empty() {
        warnings.push(warn("empty_segments", "no segments were selected"));
    }
    let resample = parse_resample(&args.resample)?;
    warnings.extend(discrete_resample_warnings(&channels, resample));
    match args.format {
        DataFormat::Json => {
            let value = extract_json(
                &args.file,
                &ld,
                &channels,
                &segments,
                args.x_axis,
                resample,
                &warnings,
            )?;
            let content = serde_json::to_string_pretty(&value).map_err(|err| err.to_string())?;
            write_output(args.output.as_deref(), &(content + "\n"))?;
        }
        DataFormat::Csv | DataFormat::Ndjson => {
            let content = extract_csv_or_ndjson(
                &ld,
                &channels,
                &segments,
                args.x_axis,
                resample,
                args.format,
            )?;
            write_output(args.output.as_deref(), &content)?;
        }
    }
    Ok(())
}

fn handle_stats(args: StatsArgs) -> Result<(), String> {
    let ld = open_ld(&args.file)?;
    let channels = resolve_channels(&ld, &args.channels)?;
    let (laps, _, _, _) = load_laps(&args.file, &ld, LapSourceArg::Auto);
    let metrics = parse_metrics(&args.metrics)?;
    match args.format {
        JsonCsvFormat::Json => print_json(&stats_json(
            &args.file, &ld, &channels, &laps, args.group, &metrics,
        )?)?,
        JsonCsvFormat::Csv => print!(
            "{}",
            stats_csv(&ld, &channels, &laps, args.group, &metrics)?
        ),
    }
    Ok(())
}

fn handle_compare_laps(args: CompareLapsArgs) -> Result<(), String> {
    let ld = open_ld(&args.file)?;
    let channels = resolve_channels(&ld, &args.channels)?;
    let (laps, _, _, _) = load_laps(&args.file, &ld, LapSourceArg::Auto);
    let lap_selectors = parse_lap_selectors(&args.laps);
    if lap_selectors.is_empty() {
        return Err("--laps must name at least one lap".into());
    }
    let resample = parse_resample(&args.resample)?;
    match args.format {
        JsonCsvFormat::Json => print_json(&compare_laps_json(
            &args.file,
            &ld,
            &channels,
            &laps,
            CompareLapsJsonParams {
                lap_selectors: &lap_selectors,
                reference_selector: &args.reference,
                axis: args.x_axis,
                resample,
            },
        )?)?,
        JsonCsvFormat::Csv => print!(
            "{}",
            compare_laps_csv(
                &ld,
                &channels,
                &laps,
                &lap_selectors,
                &args.reference,
                resample
            )?
        ),
    }
    Ok(())
}

fn handle_histogram(args: HistogramArgs) -> Result<(), String> {
    let ld = open_ld(&args.file)?;
    let channel = resolve_channel(&ld, &args.channel)?;
    let (laps, _, _, _) = load_laps(&args.file, &ld, LapSourceArg::Auto);
    let segments = select_segments(&ld, &laps, None, args.lap.as_deref(), None)?;
    let segment = segments
        .first()
        .ok_or_else(|| "no segment selected".to_string())?;
    match args.format {
        JsonCsvFormat::Json => print_json(&histogram_json(
            &args.file, &ld, channel, segment, args.bins,
        )?)?,
        JsonCsvFormat::Csv => print!("{}", histogram_csv(&ld, channel, segment, args.bins)?),
    }
    Ok(())
}

fn handle_math(args: MathArgs) -> Result<(), String> {
    let ld = open_ld(&args.file)?;
    let (laps, _, _, _) = load_laps(&args.file, &ld, LapSourceArg::Auto);
    let lap = match &args.lap {
        Some(selector) => Some(
            find_lap(&laps, selector)
                .ok_or_else(|| format!("lap not found: {selector}"))?
                .1,
        ),
        None => None,
    };
    match args.format {
        JsonCsvFormat::Json => print_json(&json!({
            "schema_version": SCHEMA_VERSION,
            "command": "math",
            "file": { "path": args.file },
            "results": evaluate_math_channels(&ld, &args.expressions, lap)?,
            "warnings": [],
        }))?,
        JsonCsvFormat::Csv => print!("{}", math_csv(&ld, &args.expressions, lap)?),
    }
    Ok(())
}

impl RunOperation {
    fn execute(
        &self,
        path: &Path,
        ld: &LdFile,
        laps: &[Lap],
        ldx: Option<&LdxFile>,
        run_warnings: &[WarningMessage],
    ) -> Result<Value, String> {
        match self {
            Self::Summary => Ok(summary_json(path, ld, laps, ldx, run_warnings)),
            Self::Channels { filter } => {
                let filter = filter.as_deref().map(str::to_ascii_lowercase);
                Ok(json!({
                    "schema_version": SCHEMA_VERSION,
                    "command": "channels",
                    "channels": filtered_channels(ld, filter.as_deref()).into_iter().map(channel_json).collect::<Vec<_>>(),
                    "warnings": [],
                }))
            }
            Self::Laps => Ok(json!({
                "schema_version": SCHEMA_VERSION,
                "command": "laps",
                "laps": laps.iter().map(lap_json).collect::<Vec<_>>(),
                "warnings": [],
            })),
            Self::Stats {
                channels,
                group,
                metrics,
            } => {
                let channels = resolve_channels(ld, channels)?;
                let metrics = parse_metrics(metrics)?;
                stats_json(path, ld, &channels, laps, *group, &metrics)
            }
            Self::Extract {
                channels,
                time,
                lap,
                laps: selected_laps,
                x_axis,
                resample,
            } => {
                let channels = resolve_channels(ld, channels)?;
                let resample = parse_resample(resample)?;
                let segments = select_segments(
                    ld,
                    laps,
                    time.as_deref(),
                    lap.as_deref(),
                    selected_laps.as_deref(),
                )?;
                let warnings = discrete_resample_warnings(&channels, resample);
                extract_json(path, ld, &channels, &segments, *x_axis, resample, &warnings)
            }
            Self::CompareLaps {
                channels,
                laps: selected_laps,
                reference,
                x_axis,
                resample,
            } => {
                let channels = resolve_channels(ld, channels)?;
                let lap_selectors = selected_laps.selectors();
                let resample = parse_resample(resample)?;
                compare_laps_json(
                    path,
                    ld,
                    &channels,
                    laps,
                    CompareLapsJsonParams {
                        lap_selectors: &lap_selectors,
                        reference_selector: reference,
                        axis: *x_axis,
                        resample,
                    },
                )
            }
            Self::Histogram { channel, lap, bins } => {
                let channel = resolve_channel(ld, channel)?;
                let segments = select_segments(ld, laps, None, lap.as_deref(), None)?;
                let segment = segments
                    .first()
                    .ok_or_else(|| "histogram operation selected no segment".to_string())?;
                histogram_json(path, ld, channel, segment, *bins)
            }
            Self::Math {
                expr,
                expressions,
                lap,
            } => {
                let expressions = match (expressions, expr) {
                    (Some(expressions), _) => expressions.clone(),
                    (None, Some(expr)) => vec![expr.clone()],
                    (None, None) => {
                        return Err("math operation requires `expr` or `expressions`".into());
                    }
                };
                let lap = match lap.as_deref() {
                    Some(selector) => Some(
                        find_lap(laps, selector)
                            .ok_or_else(|| format!("lap not found: {selector}"))?
                            .1,
                    ),
                    None => None,
                };
                Ok(json!({
                    "schema_version": SCHEMA_VERSION,
                    "command": "math",
                    "results": evaluate_math_channels(ld, &expressions, lap)?,
                    "warnings": [],
                }))
            }
        }
    }
}

fn handle_run(args: RunArgs) -> Result<(), String> {
    if args.format != TextJsonFormat::Json {
        return Err("run currently supports --format json only".into());
    }
    let raw = fs::read_to_string(&args.spec)
        .map_err(|err| format!("could not read {}: {err}", args.spec.display()))?;
    let spec: AnalysisRunSpec =
        serde_json::from_str(&raw).map_err(|err| format!("invalid spec JSON: {err}"))?;
    let path = PathBuf::from(&spec.file);
    let ld = open_ld(&path)?;
    let (laps, ldx, warnings, _) = load_laps(&path, &ld, LapSourceArg::Auto);

    let mut results = Vec::new();
    for operation in &spec.operations {
        results.push(operation.execute(&path, &ld, &laps, ldx.as_ref(), &warnings)?);
    }

    print_json(&json!({
        "schema_version": SCHEMA_VERSION,
        "command": "run",
        "spec": args.spec,
        "file": { "path": path },
        "results": results,
        "warnings": [],
    }))
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::Summary(args) => handle_summary(args),
        Commands::Channels(args) => handle_channels(args),
        Commands::Laps(args) => handle_laps(args),
        Commands::Extract(args) => handle_extract(args),
        Commands::Stats(args) => handle_stats(args),
        Commands::CompareLaps(args) => handle_compare_laps(args),
        Commands::Histogram(args) => handle_histogram(args),
        Commands::Math(args) => handle_math(args),
        Commands::Run(args) => handle_run(args),
        Commands::InstallSkill(args) => match install_skill(args) {
            Ok(paths) => {
                println!("Installed i3rs CLI skill to:");
                for path in paths {
                    println!("{}", path.display());
                }
                Ok(())
            }
            Err(err) => Err(err),
        },
    }
}

fn is_known_subcommand(arg: &str) -> bool {
    if matches!(arg, "help" | "--help" | "-h" | "--version" | "-V") {
        return true;
    }

    Cli::command().get_subcommands().any(|subcommand| {
        subcommand.get_name() == arg || subcommand.get_all_aliases().any(|alias| alias == arg)
    })
}

fn normalized_args() -> Vec<String> {
    let mut args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "help" {
        args[1] = "--help".into();
    } else if args.len() > 1 && !args[1].starts_with('-') && !is_known_subcommand(&args[1]) {
        args.insert(1, "summary".into());
    }
    args
}

fn print_help_and_legacy_note() -> Result<(), String> {
    let mut cmd = Cli::command();
    cmd.print_long_help().map_err(|err| err.to_string())?;
    println!("\nLegacy:\n  i3rs <file.ld>    Parse a MoTeC .ld log and print the text summary.");
    Ok(())
}

fn print_help_to_stderr_and_legacy_note() -> Result<(), String> {
    let mut cmd = Cli::command();
    let mut help = Vec::new();
    cmd.write_long_help(&mut help)
        .map_err(|err| err.to_string())?;
    eprint!("{}", String::from_utf8_lossy(&help));
    eprintln!("\nLegacy:\n  i3rs <file.ld>    Parse a MoTeC .ld log and print the text summary.");
    Ok(())
}

fn main() {
    let args = normalized_args();
    if args.len() == 1 {
        if let Err(err) = print_help_to_stderr_and_legacy_note() {
            eprintln!("Error: {err}");
        }
        process::exit(1);
    }
    if args
        .get(1)
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        if let Err(err) = print_help_and_legacy_note() {
            eprintln!("Error: {err}");
            process::exit(1);
        }
        return;
    }

    let cli = Cli::parse_from(args);
    if let Err(err) = run(cli) {
        eprintln!("Error: {err}");
        process::exit(1);
    }
}
