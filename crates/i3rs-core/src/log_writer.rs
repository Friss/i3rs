//! Lossless full-log copies and explicit fixed-rate cropped logs, with validated sidecars.
//! Original binary/configuration metadata is retained. Only logged channels are writable.
use crate::{
    Interpolation, LdFile,
    output::{OutputPlan, OutputSeries},
    ranges::RecordingAnalysis,
};
use std::path::{Path, PathBuf};

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn put32(b: &mut [u8], i: usize, v: usize) -> Result<(), String> {
    let v = u32::try_from(v).map_err(|_| "Log exceeds 32-bit format size")?;
    b[i..i + 4].copy_from_slice(&v.to_le_bytes());
    Ok(())
}
fn text(b: &mut [u8], i: usize, n: usize, v: &str) -> Result<(), String> {
    if v.len() > n || v.contains('\0') {
        return Err(format!("Log detail exceeds {} UTF-8 bytes", n));
    }
    b[i..i + n].fill(0);
    b[i..i + v.len()].copy_from_slice(v.as_bytes());
    Ok(())
}
/// None preserves original channel bytes/rates. Some resamples to one integral Hz
/// grid anchored at start, rebased to zero. Continuous = linear; states = previous.
/// Display transforms and derived channels never enter this raw-log adapter.
pub fn encode_log(
    ld: &LdFile,
    analysis: &RecordingAnalysis,
    selection: Option<OutputPlan>,
) -> Result<(Vec<u8>, RecordingAnalysis), String> {
    analysis.validate()?;
    if analysis.recording != ld.recording_id()
        || (analysis.duration - ld.duration_secs()).abs() > 1e-6
    {
        return Err("Analysis does not belong to this log".into());
    }
    let mut bytes = ld.source_bytes().to_vec();
    text(&mut bytes, 0x9e, 64, &analysis.details.driver)?;
    text(&mut bytes, 0xde, 64, &analysis.details.vehicle)?;
    text(&mut bytes, 0x15e, 64, &analysis.details.venue)?;
    // The complete notes/custom details live in the paired analysis sidecar.
    let event = u32_at(&bytes, 0x24) as usize;
    if event > 0 && event + 1156 <= bytes.len() {
        text(&mut bytes, event, 64, &analysis.details.event)?;
        text(&mut bytes, event + 128, 1024, &analysis.details.notes)?;
        let venue = u32_at(&bytes, event + 1152) as usize;
        if venue > 0 && venue + 1102 <= bytes.len() {
            text(&mut bytes, venue, 64, &analysis.details.venue)?;
            let vehicle = u32_at(&bytes, venue + 1098) as usize;
            if vehicle > 0 && vehicle + 64 <= bytes.len() {
                text(&mut bytes, vehicle, 64, &analysis.details.vehicle)?;
            }
        }
    }
    let mut result = analysis.clone();
    if let Some(plan) = selection {
        let n = plan.count()?;
        if plan.end > ld.duration_secs() + 1e-9 || plan.rate.fract() != 0. || plan.rate > 65535. {
            return Err("Cropped logs require integral 1–65535 Hz within recording bounds".into());
        }
        let additional = n
            .checked_mul(ld.channels.len())
            .and_then(|v| v.checked_mul(8))
            .ok_or("Output size overflow")?;
        if bytes.len().saturating_add(additional) > u32::MAX as usize {
            return Err("Log exceeds 32-bit format size".into());
        }
        if n.saturating_mul(ld.channels.len()) > 20_000_000 {
            return Err("Cropped log exceeds 20 million samples; reduce rate or scope".into());
        }
        let mut meta = u32_at(&bytes, 8) as usize;
        for ch in &ld.channels {
            if meta == 0 || meta + 212 > bytes.len() {
                return Err("Invalid channel metadata chain".into());
            }
            let next = u32_at(&bytes, meta + 4) as usize;
            let data = ld
                .read_channel_data(ch)
                .ok_or_else(|| format!("Cannot decode {}", ch.name))?;
            let s = OutputSeries {
                name: ch.name.clone(),
                unit: ch.unit.clone(),
                data,
                rate: ch.freq,
                precision: 9,
                interpolation: if !ch.enum_labels.is_empty() || crate::is_state_channel(&ch.name) {
                    Interpolation::Previous
                } else {
                    Interpolation::Linear
                },
                scale: 1.,
                offset: 0.,
            };
            // Do not leave out-of-scope source samples recoverable in the copy.
            // Preserve pointer addresses for configuration/enum/name extensions.
            let original_ptr = u32_at(ld.source_bytes(), meta + 8) as usize;
            let original_len = (ch.n_data as usize)
                .checked_mul(
                    ch.data_type
                        .bytes_per_sample()
                        .ok_or("Unsupported source data type")?,
                )
                .ok_or("Source sample size overflow")?;
            let original_end = original_ptr
                .checked_add(original_len)
                .ok_or("Source sample offset overflow")?;
            if original_end > ld.source_bytes().len() {
                return Err("Truncated source sample data".into());
            }
            bytes[original_ptr..original_end].fill(0);
            let ptr = bytes.len();
            // Store floating samples in engineering units. Legacy i2 does not
            // resolve continuous f64 channels with nonzero scaling exponents.
            for i in 0..n {
                let value = s.value(plan.time(i)).unwrap_or(f64::NAN);
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            put32(&mut bytes, meta + 8, ptr)?;
            put32(&mut bytes, meta + 12, n)?;
            for (offset, v) in [
                (18, 8u16),
                (20, 8),
                (22, plan.rate as u16),
                (24, 0),
                (26, 1),
                (28, 1),
                (30, 0),
            ] {
                bytes[meta + offset..meta + offset + 2].copy_from_slice(&v.to_le_bytes());
            }
            // Retain dedicated precision for consumers that support it. Older
            // i2 versions may display these engineering-unit floats rounded.
            bytes[meta + 100] = ch.display_decimals;
            bytes[meta + 101] = 3;
            // Invalidity is now represented by NaN on the output grid.
            put32(&mut bytes, meta + 116, 0)?;
            meta = next;
        }
        put32(&mut bytes, 0x42, 1_000_000)?;
        result.duration = n as f64 / plan.rate;
        result.ranges.retain_mut(|r| {
            r.start = r.start.max(plan.start) - plan.start;
            r.end = r.end.min(plan.end) - plan.start;
            if !analysis.ranges.iter().any(|original| {
                original.id == r.id && original.start >= plan.start && original.end <= plan.end
            }) {
                r.reaction_time = None;
            }
            r.end > r.start
        });
        result.markers.retain_mut(|m| {
            let keep = m.time >= plan.start && m.time < plan.end;
            m.time -= plan.start;
            m.distance_m = None;
            keep
        });
        result.drag_runs.retain(|id, _| {
            analysis
                .ranges
                .iter()
                .any(|r| &r.id == id && r.start >= plan.start && r.end <= plan.end)
        });
        // Sample-index sections and automatically generated GPS boundaries cannot
        // be copied onto a different clock. Geographic track definition is retained.
        result.sectors.clear();
        result.gps_generation = None;
        if !result
            .ranges
            .iter()
            .any(|r| Some(&r.id) == result.selected_range.as_ref())
        {
            result.selected_range = None;
        }
    }
    let reopened = LdFile::from_bytes(bytes.clone())?;
    if reopened.channels.len() != ld.channels.len()
        || reopened
            .channels
            .iter()
            .zip(&ld.channels)
            .any(|(a, b)| a.name != b.name || a.unit != b.unit)
    {
        return Err("Encoded log failed channel metadata validation".into());
    }
    result.recording = reopened.recording_id().into();
    result.duration = reopened.duration_secs();
    result.validate()?;
    Ok((bytes, result))
}

/// i2 layer hierarchy and observed Stage/Run markers. Generic ranges use a private
/// JSON detail readable by i3rs; full identity/type/trust/comments live in JSON.
pub fn analysis_ldx(a: &RecordingAnalysis) -> String {
    use crate::ranges::RangeKind;
    use std::fmt::Write;
    let esc = |s: &str| quick_xml::escape::escape(s).into_owned();
    let mut xml = String::from(
        "<?xml version=\"1.0\"?><LDXFile DefaultLocale=\"C\" Version=\"1.6\"><Layers><Layer><MarkerBlock>",
    );
    let stages = a
        .ranges
        .iter()
        .filter(|r| r.kind == RangeKind::Stage)
        .collect::<Vec<_>>();
    if !stages.is_empty() {
        xml.push_str("<MarkerGroup Name=\"StageMarkers\" Index=\"10\">");
        for (i, r) in stages.iter().enumerate() {
            for (class, t) in [("STGSTRT", r.start), ("STGEND", r.end)] {
                let _ = write!(
                    xml,
                    "<Marker Version=\"100\" ClassName=\"{class}\" Name=\"i3rs.{i}\" Flags=\"77\" Time=\"{:.17e}\"/>",
                    t * 1_000_000.
                );
            }
        }
        xml.push_str("</MarkerGroup>");
    }
    let runs = a
        .ranges
        .iter()
        .filter(|r| r.kind == RangeKind::Run)
        .collect::<Vec<_>>();
    if !runs.is_empty() {
        xml.push_str("<MarkerGroup Name=\"Outings:Runs\" Index=\"1\">");
    }
    for r in &runs {
        let _ = write!(
            xml,
            "<Marker Version=\"100\" ClassName=\"SOR\" Name=\"\" Flags=\"13\" Time=\"{:.17e}\"/>",
            r.start * 1_000_000.
        );
    }
    if !runs.is_empty() {
        xml.push_str("</MarkerGroup>");
    }
    xml.push_str("</MarkerBlock><RangeBlock/></Layer><Details>");
    let mut details = a.details.custom.clone();
    let pairs = a
        .ranges
        .iter()
        .map(|r| (r.start, r.end))
        .collect::<Vec<_>>();
    details.insert(
        "i3rs Ranges (s)".into(),
        serde_json::to_string(&pairs).expect("validated finite range bounds"),
    );

    // Recalculate result metadata rather than reusing stale source outing claims.
    for key in [
        "Reaction Time",
        "Run Name",
        "Time 1320 ft",
        "Total Laps",
        "Fastest Lap",
        "Fastest Time",
    ] {
        details.remove(key);
    }
    for (key, value) in [
        ("Driver", &a.details.driver),
        ("Vehicle Id", &a.details.vehicle),
        ("Venue", &a.details.venue),
        ("Event", &a.details.event),
        ("Short Comment", &a.details.notes),
    ] {
        details.insert(key.into(), value.clone());
    }
    let competition = a
        .ranges
        .iter()
        .filter(|r| r.kind.competition())
        .collect::<Vec<_>>();
    details.insert("Total Laps".into(), competition.len().to_string());
    if let Some((i, r)) = competition
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (a.end - a.start).total_cmp(&(b.end - b.start)))
    {
        details.insert(
            "Fastest Time".into(),
            crate::format_duration(r.end - r.start),
        );
        details.insert("Fastest Lap".into(), (i + 1).to_string());
    }
    if let [r] = runs.as_slice() {
        details.insert("Run Name".into(), r.name.clone());
        details.insert("Time 1320 ft".into(), format!("{:.9}", r.end - r.start));
        if let Some(t) = r.reaction_time {
            details.insert("Reaction Time".into(), format!("{t:.9}"));
        }
    }
    for (key, value) in details {
        let _ = write!(
            xml,
            "<String Id=\"{}\" Value=\"{}\"/>",
            esc(&key),
            esc(&value)
        );
    }
    xml.push_str("</Details></Layers></LDXFile>");
    xml
}

/// Publish a log and paired analysis/LDX files. Existing files are backed up before
/// replacement; failures roll back all published files. Backups remain recoverable.
/// The caller must release/reopen any mapped source before further edits.
pub fn write_log(
    path: &Path,
    ld: &LdFile,
    analysis: &RecordingAnalysis,
    selection: Option<OutputPlan>,
) -> Result<Option<PathBuf>, String> {
    if !path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("ld"))
    {
        return Err("Log output filename must end in .ld".into());
    }
    let (bytes, a) = encode_log(ld, analysis, selection)?;
    let json = serde_json::to_vec_pretty(&a).map_err(|e| e.to_string())?;
    let ldx = analysis_ldx(&a);
    let destinations = [
        path.to_path_buf(),
        path.with_extension("i3rs.json"),
        path.with_extension("ldx"),
    ];
    let payloads = [bytes, json, ldx.into_bytes()];
    publish_outputs(path, &destinations, &payloads)
}

/// Shared recoverable publication for binary format adapters.
pub(crate) fn publish_outputs(
    path: &Path,
    destinations: &[PathBuf],
    payloads: &[Vec<u8>],
) -> Result<Option<PathBuf>, String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let staging = parent.join(format!(".i3rs-output-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let backup = parent.join(format!("i3rs-backup-{nonce}"));
    let mut backed_up = vec![];
    let mut published = vec![];
    let result = (|| -> Result<(), String> {
        use std::io::Write;
        for (i, payload) in payloads.iter().enumerate() {
            let mut f =
                std::fs::File::create(staging.join(i.to_string())).map_err(|e| e.to_string())?;
            f.write_all(payload).map_err(|e| e.to_string())?;
            f.sync_all().map_err(|e| e.to_string())?;
        }
        if destinations.iter().any(|p| p.exists()) {
            std::fs::create_dir(&backup).map_err(|e| e.to_string())?;
            for (i, p) in destinations.iter().enumerate() {
                if p.exists() {
                    if !p.is_file() || p.is_symlink() {
                        return Err("Refusing to replace non-regular output file".into());
                    }
                    std::fs::copy(p, backup.join(i.to_string())).map_err(|e| e.to_string())?;
                    backed_up.push(i);
                }
            }
            let manifest = destinations
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{i}: {}\n", p.display()))
                .collect::<String>();
            std::fs::write(backup.join("RESTORE.txt"), manifest).map_err(|e| e.to_string())?;
        }
        for (i, p) in destinations.iter().enumerate() {
            std::fs::rename(staging.join(i.to_string()), p).map_err(|e| e.to_string())?;
            published.push(i);
        }
        Ok(())
    })();
    if result.is_err() {
        for i in &published {
            let p = &destinations[*i];
            if backed_up.contains(i) {
                let _ = std::fs::copy(backup.join(i.to_string()), p);
            } else {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
    result.map_err(|e| format!("{e}; recovery files: {}", backup.display()))?;
    Ok(backup.exists().then_some(backup))
}
