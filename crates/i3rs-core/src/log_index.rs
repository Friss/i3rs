//! Incremental metadata/range index. Only lap/beacon channels are decoded for range discovery.
use crate::{LdFile, detect_laps, ranges::RecordingAnalysis};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexedLog {
    pub path: PathBuf,
    pub size: u64,
    pub modified: u128,
    pub driver: String,
    pub venue: String,
    pub vehicle: String,
    pub date: String,
    pub time: String,
    pub event: String,
    pub duration: f64,
    pub channels: usize,
    pub analysis: RecordingAnalysis,
}
impl IndexedLog {
    pub fn matches(&self, query: &str) -> bool {
        let text = format!(
            "{} {} {} {} {} {} {} {}",
            self.path.display(),
            self.driver,
            self.venue,
            self.vehicle,
            self.date,
            self.time,
            self.event,
            self.analysis
                .ranges
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();
        query
            .split_whitespace()
            .all(|q| text.contains(&q.to_lowercase()))
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct LogIndex {
    pub folders: Vec<PathBuf>,
    pub logs: BTreeMap<PathBuf, IndexedLog>,
}
impl LogIndex {
    /// Rebuild a folder recursively, keeping other shortcuts. Per-file errors are
    /// visible and do not prevent valid recordings from being indexed.
    pub fn scan(&mut self, folder: &Path) -> Vec<String> {
        let mut errors = vec![];
        let mut paths = vec![];
        fn visit(dir: &Path, paths: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("{}: {e}", dir.display()));
                    return;
                }
            };
            for entry in entries {
                match entry {
                    Ok(e) => {
                        let p = e.path();
                        if p.is_symlink() {
                            continue;
                        }
                        if p.is_dir() {
                            if !p
                                .file_name()
                                .is_some_and(|s| s.to_string_lossy().starts_with('.'))
                            {
                                visit(&p, paths, errors);
                            }
                        } else if p.extension().is_some_and(|s| s.eq_ignore_ascii_case("ld")) {
                            paths.push(p);
                        }
                    }
                    Err(e) => errors.push(e.to_string()),
                }
            }
        }
        visit(folder, &mut paths, &mut errors);
        self.logs
            .retain(|p, _| !p.starts_with(folder) || paths.contains(p));
        for path in paths {
            let result = (|| -> Result<IndexedLog, String> {
                let md = std::fs::metadata(&path).map_err(|e| e.to_string())?;
                let modified = md
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                let modified = modified
                    ^ [path.with_extension("ldx"), path.with_extension("i3rs.json")]
                        .iter()
                        .fold(0, |stamp, p| {
                            stamp
                                ^ std::fs::metadata(p).ok().map_or(0, |m| {
                                    m.modified()
                                        .ok()
                                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                        .map_or(0, |d| d.as_nanos())
                                        .wrapping_mul(m.len() as u128 + 1)
                                })
                        });
                if let Some(cached) = self
                    .logs
                    .get(&path)
                    .filter(|c| c.size == md.len() && c.modified == modified)
                {
                    return Ok(cached.clone());
                }
                let ld = LdFile::open(&path)?;
                let sidecar = path.with_extension("i3rs.json");
                let analysis = if sidecar.exists() {
                    let mut a: RecordingAnalysis =
                        serde_json::from_slice(&std::fs::read(sidecar).map_err(|e| e.to_string())?)
                            .map_err(|e| e.to_string())?;
                    a.migrate_sample_clock(&ld)?;
                    a
                } else {
                    let ldx = crate::find_ldx_for_ld(&path);
                    let specialty = ldx
                        .as_ref()
                        .map(|x| x.specialty_analysis(ld.recording_id().into(), ld.duration_secs()))
                        .transpose()?
                        .flatten();
                    specialty.unwrap_or_else(|| {
                        let laps = ldx
                            .as_ref()
                            .filter(|x| !x.laps.is_empty())
                            .map(|x| {
                                x.laps
                                    .iter()
                                    .map(|l| crate::Lap {
                                        number: l.number,
                                        name: format!("Lap {}", l.number),
                                        start_time: l.start_time,
                                        end_time: l.end_time,
                                    })
                                    .collect()
                            })
                            .unwrap_or_else(|| detect_laps(&ld));
                        let mut a = RecordingAnalysis::from_laps(
                            ld.recording_id().into(),
                            ld.duration_secs(),
                            &laps,
                        );
                        a.details.driver = ld.session.driver.clone();
                        a.details.vehicle = ld.session.vehicle_id.clone();
                        a.details.venue = ld.session.venue.clone();
                        a.details.event = ld.event.event_name.clone();
                        a.details.notes = ld.event.comment.clone();
                        a
                    })
                };
                Ok(IndexedLog {
                    path: path.clone(),
                    size: md.len(),
                    modified,
                    driver: analysis.details.driver.clone(),
                    venue: analysis.details.venue.clone(),
                    vehicle: analysis.details.vehicle.clone(),
                    event: analysis.details.event.clone(),
                    date: ld.session.date.clone(),
                    time: ld.session.time.clone(),
                    duration: ld.duration_secs(),
                    channels: ld.channels.len(),
                    analysis,
                })
            })();
            match result {
                Ok(log) => {
                    self.logs.insert(path, log);
                }
                Err(e) => {
                    self.logs.remove(&path);
                    errors.push(format!("{}: {e}", path.display()));
                }
            }
        }
        if !self.folders.contains(&folder.to_path_buf()) {
            self.folders.push(folder.to_path_buf());
        }
        errors
    }
}
