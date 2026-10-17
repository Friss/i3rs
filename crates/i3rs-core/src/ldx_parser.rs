//! Parser for MoTeC .ldx sidecar XML files.
//!
//! The .ldx file sits alongside the .ld file and contains lap timing
//! information written by MoTeC i2 or the ECU logging system.

use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::BTreeMap;
use std::path::Path;

/// Parsed lap timing data from a .ldx sidecar file.
#[derive(Debug, Clone, Default)]
pub struct LdxFile {
    pub details: BTreeMap<String, (String, String)>,
    pub markers: Vec<LdxMarker>,
    pub total_laps: Option<u32>,
    pub fastest_time: Option<String>,
    pub fastest_lap: Option<u32>,
    /// Individual lap markers if present (start_time, end_time in seconds).
    pub laps: Vec<LdxLap>,
}

#[derive(Debug, Clone)]
pub struct LdxMarker {
    pub group: String,
    pub class: String,
    pub name: String,
    /// LDX Marker timestamps are microseconds from recording start.
    pub time: f64,
}

/// A single lap entry from .ldx marker data.
#[derive(Debug, Clone)]
pub struct LdxLap {
    pub number: u32,
    pub start_time: f64,
    pub end_time: f64,
}

impl LdxFile {
    /// Try to open and parse a .ldx file.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Failed to read .ldx file: {}", e))?;
        Self::parse(&content)
    }

    /// Parse .ldx XML content.
    pub fn parse(xml: &str) -> Result<Self, String> {
        let mut ldx = LdxFile::default();
        let mut reader = Reader::from_str(xml);
        let mut group = String::new();

        loop {
            match reader.read_event() {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"MarkerGroup" => {
                    group = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == b"Name")
                        .and_then(|a| {
                            a.decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )
                            .ok()
                            .map(|s| s.into_owned())
                        })
                        .unwrap_or_default();
                }
                Ok(Event::End(ref e)) if e.name().as_ref() == b"MarkerGroup" => group.clear(),
                Ok(Event::Empty(ref e)) if e.name().as_ref() == b"Marker" => {
                    let attrs: BTreeMap<_, _> = e
                        .attributes()
                        .flatten()
                        .filter_map(|a| {
                            a.decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )
                            .ok()
                            .map(|v| (a.key.as_ref().to_vec(), v.into_owned()))
                        })
                        .collect();
                    if let Some(time) = attrs
                        .get(b"Time".as_slice())
                        .and_then(|v| v.parse::<f64>().ok())
                        .filter(|t| t.is_finite())
                    {
                        ldx.markers.push(LdxMarker {
                            group: group.clone(),
                            class: attrs
                                .get(b"ClassName".as_slice())
                                .cloned()
                                .unwrap_or_default(),
                            name: attrs.get(b"Name".as_slice()).cloned().unwrap_or_default(),
                            time: time / 1_000_000.,
                        });
                    }
                }
                Ok(Event::Empty(ref e)) if matches!(e.name().as_ref(), b"String" | b"Numeric") => {
                    let mut id = String::new();
                    let mut value = String::new();
                    let mut unit = String::new();
                    for attr in e.attributes().flatten() {
                        let decoded = attr
                            .decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )
                            .map_err(|e| e.to_string())?
                            .into_owned();
                        match attr.key.as_ref() {
                            b"Id" => id = decoded,
                            b"Value" => value = decoded,
                            b"Unit" => unit = decoded,
                            _ => {}
                        }
                    }
                    ldx.details.insert(id.clone(), (value.clone(), unit));
                    match id.as_str() {
                        "Total Laps" => ldx.total_laps = value.parse().ok(),
                        "Fastest Time" => ldx.fastest_time = Some(value),
                        "Fastest Lap" => ldx.fastest_lap = value.parse().ok(),
                        _ => {}
                    }
                }
                Ok(Event::Empty(ref e)) if e.name().as_ref() == b"MarkerPair" => {
                    let mut start = 0.0_f64;
                    let mut end = 0.0_f64;
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"Time" => {
                                start = String::from_utf8_lossy(&attr.value).parse().unwrap_or(0.0);
                            }
                            b"EndTime" => {
                                end = String::from_utf8_lossy(&attr.value).parse().unwrap_or(0.0);
                            }
                            _ => {}
                        }
                    }
                    let number = ldx.laps.len() as u32;
                    ldx.laps.push(LdxLap {
                        number,
                        start_time: start,
                        end_time: end,
                    });
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("XML parse error: {}", e)),
                _ => {}
            }
        }

        // i3rs generic ranges are an explicitly named private detail. They do
        // not masquerade as MoTeC MarkerPair timestamps or alter i2 outings.
        if let Some((json, _)) = ldx.details.get("i3rs Ranges (s)") {
            let ranges: Vec<(f64, f64)> =
                serde_json::from_str(json).map_err(|e| format!("Invalid i3rs LDX ranges: {e}"))?;
            if ranges.iter().any(|(start, end)| {
                !start.is_finite() || !end.is_finite() || *start < 0. || end <= start
            }) {
                return Err("Invalid i3rs LDX range bounds".into());
            }
            ldx.laps = ranges
                .into_iter()
                .enumerate()
                .map(|(i, (start_time, end_time))| LdxLap {
                    number: i as u32,
                    start_time,
                    end_time,
                })
                .collect();
        }
        Ok(ldx)
    }

    /// Import the observed Standard Run / Stage metadata. Ordinary Circuit
    /// detection stays unchanged when no specialty marker group is present.
    pub fn specialty_analysis(
        &self,
        recording: String,
        duration: f64,
    ) -> Result<Option<crate::ranges::RecordingAnalysis>, String> {
        use crate::ranges::{AnalysisRange, RangeKind, RecordingAnalysis};
        let get = |name: &str| self.details.get(name).map(|v| v.0.as_str());
        let seconds = |name: &str| get(name).and_then(parse_duration);
        let mut analysis = RecordingAnalysis {
            recording,
            duration,
            ..Default::default()
        };
        let runs: Vec<_> = self
            .markers
            .iter()
            .filter(|m| m.group == "Outings:Runs" && m.class == "SOR" && m.time >= 0.)
            .collect();
        let mut competition = Vec::new();
        if !runs.is_empty() {
            for (i, m) in runs.iter().enumerate() {
                let end = seconds("Time 1320 ft")
                    .filter(|_| runs.len() == 1)
                    .map(|t| m.time + t)
                    .unwrap_or_else(|| runs.get(i + 1).map_or(duration, |m| m.time));
                competition.push(AnalysisRange {
                    id: format!("ldx-run-{i}"),
                    name: if runs.len() == 1 {
                        get("Run Name").unwrap_or("Run 1").into()
                    } else {
                        format!("Run {}", i + 1)
                    },
                    start: m.time,
                    end,
                    kind: RangeKind::Run,
                    trust: Default::default(),
                    comment: String::new(),
                    reaction_time: seconds("Reaction Time").filter(|_| runs.len() == 1),
                });
            }
            if runs.len() == 1 {
                analysis.drag_runs.insert(
                    "ldx-run-0".into(),
                    crate::drag::RunDetails {
                        elapsed: crate::drag::MILESTONES_FT
                            .map(|ft| seconds(&format!("Time {ft} ft"))),
                    },
                );
            }
            analysis.drag_setup.ratios = (1..=16)
                .map_while(|i| {
                    get(&format!("Gear {i}"))
                        .and_then(|v| v.parse::<f64>().ok())
                        .filter(|v| v.is_finite() && *v > 0.)
                })
                .collect();
            analysis.corrections.fastest_pair = None;
            analysis
                .corrections
                .speed_priority
                .insert(0, "Wheel Speed - Front".into());
        } else {
            for m in self
                .markers
                .iter()
                .filter(|m| m.group == "StageMarkers" && m.class == "STGSTRT")
            {
                let end = self
                    .markers
                    .iter()
                    .filter(|e| {
                        e.group == m.group
                            && e.class == "STGEND"
                            && e.name == m.name
                            && e.time > m.time
                    })
                    .min_by(|a, b| a.time.total_cmp(&b.time))
                    .ok_or("Stage start has no matching end marker")?;
                competition.push(AnalysisRange {
                    id: format!("ldx-stage-{}", competition.len()),
                    name: format!("Stage {}", competition.len() + 1),
                    start: m.time,
                    end: end.time,
                    kind: RangeKind::Stage,
                    trust: Default::default(),
                    comment: String::new(),
                    reaction_time: None,
                });
            }
            if competition.is_empty() {
                return Ok(None);
            }
            analysis.corrections.fastest_pair = None;
            analysis
                .corrections
                .speed_priority
                .insert(0, "Ground Speed".into());
        }
        competition.sort_by(|a, b| a.start.total_cmp(&b.start));
        let mut previous = 0.;
        let is_drag = !runs.is_empty();
        for range in competition {
            if range.start > previous {
                analysis.ranges.push(AnalysisRange {
                    id: format!("ldx-transport-{}", analysis.ranges.len()),
                    name: if is_drag {
                        "Staging".into()
                    } else {
                        format!(
                            "Transport {}",
                            analysis
                                .ranges
                                .iter()
                                .filter(|r| r.kind == RangeKind::Transport)
                                .count()
                                + 1
                        )
                    },
                    start: previous,
                    end: range.start,
                    kind: RangeKind::Transport,
                    trust: Default::default(),
                    comment: String::new(),
                    reaction_time: None,
                });
            }
            previous = range.end;
            if analysis.selected_range.is_none() {
                analysis.selected_range = Some(range.id.clone());
            }
            analysis.ranges.push(range);
        }
        if previous < duration {
            analysis.ranges.push(AnalysisRange {
                id: "ldx-transport-post".into(),
                name: if is_drag {
                    "Post".into()
                } else {
                    format!(
                        "Transport {}",
                        analysis
                            .ranges
                            .iter()
                            .filter(|r| r.kind == RangeKind::Transport)
                            .count()
                            + 1
                    )
                },
                start: previous,
                end: duration,
                kind: RangeKind::Transport,
                trust: Default::default(),
                comment: String::new(),
                reaction_time: None,
            });
        }
        analysis.details.driver = get("Driver").unwrap_or_default().into();
        analysis.details.venue = get("Venue").unwrap_or_default().into();
        analysis.details.vehicle = get("Vehicle Id").unwrap_or_default().into();
        analysis.details.event = get("Event").unwrap_or_default().into();
        analysis.details.custom = self
            .details
            .iter()
            .map(|(k, (v, u))| (k.clone(), format!("{v} {u}").trim().into()))
            .collect();
        analysis.validate()?;
        Ok(Some(analysis))
    }
}

fn parse_duration(value: &str) -> Option<f64> {
    let mut result = 0.;
    for part in value.split(':') {
        result = result * 60. + part.parse::<f64>().ok()?;
    }
    (result.is_finite() && result >= 0.).then_some(result)
}

/// Try to find and load the .ldx sidecar for a given .ld path.
pub fn find_ldx_for_ld(ld_path: &Path) -> Option<LdxFile> {
    let ldx_path = ld_path.with_extension("ldx");
    if ldx_path.exists() {
        LdxFile::open(&ldx_path).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_ldx() {
        let xml = r#"<?xml version="1.0"?>
<LDXFile Locale="English_United States.1252" DefaultLocale="C" Version="1.6">
 <Layers>
  <Details>
   <String Id="Total Laps" Value="1"/>
   <String Id="Fastest Time" Value="15:49.610"/>
   <String Id="Fastest Lap" Value="0"/>
  </Details>
 </Layers>
</LDXFile>"#;

        let ldx = LdxFile::parse(xml).unwrap();
        assert_eq!(ldx.total_laps, Some(1));
        assert_eq!(ldx.fastest_time.as_deref(), Some("15:49.610"));
        assert_eq!(ldx.fastest_lap, Some(0));
    }

    #[test]
    fn drag_markers_import_run_slip_ratios_and_transport() {
        let ldx = LdxFile::parse(
            r#"<LDXFile><MarkerGroup Name="Outings:Runs">
          <Marker ClassName="SOR" Time="10000000"/>
          <Marker ClassName="SOC" Time="-400000"/>
        </MarkerGroup><Details>
          <String Id="Run Name" Value="Q1"/>
          <String Id="Driver" Value="A &amp; B"/>
          <String Id="Reaction Time" Value="00.400"/>
          <String Id="Time 60 ft" Value="01.200"/>
          <String Id="Time 330 ft" Value="02.810"/>
          <String Id="Time 1320 ft" Value="06.138"/>
          <Numeric Id="Gear 1" Value="1.6"/>
          <Numeric Id="Gear 2" Value="1.28"/>
          <Numeric Id="Gear 3" Value="1"/>
          <Numeric Id="Gear 4" Value="0"/>
        </Details></LDXFile>"#,
        )
        .unwrap();
        let a = ldx
            .specialty_analysis("sample".into(), 20.)
            .unwrap()
            .unwrap();
        use crate::ranges::{RangeKind, Trust};
        assert_eq!(
            a.ranges.iter().map(|r| r.kind).collect::<Vec<_>>(),
            [RangeKind::Transport, RangeKind::Run, RangeKind::Transport]
        );
        let run = &a.ranges[1];
        assert_eq!(run.name, "Q1");
        assert_eq!(run.trust, Trust::Unknown);
        assert_eq!(run.start, 10.);
        assert!((run.end - 16.138).abs() < 1e-9);
        assert_eq!(run.reaction_time, Some(0.4));
        assert_eq!(
            a.drag_runs[&run.id].elapsed,
            [Some(1.2), Some(2.81), None, None, Some(6.138)]
        );
        assert_eq!(a.drag_setup.ratios, [1.6, 1.28, 1.]);
        assert_eq!(a.details.driver, "A & B");
        assert_eq!(a.selected_range.as_deref(), Some(run.id.as_str()));
    }

    #[test]
    fn stage_markers_import_only_paired_stages_with_transport() {
        let ldx = LdxFile::parse(
            r#"<LDXFile><MarkerGroup Name="StageMarkers">
          <Marker ClassName="STGSTRT" Name="Manual.8" Flags="77" Time="7984853.5"/>
          <Marker ClassName="STGEND" Name="Manual.8" Time="141984848"/>
          <Marker ClassName="STGSTRT" Name="Manual.9" Flags="333" Time="157841872"/>
          <Marker ClassName="STGEND" Name="Manual.9" Time="277841856"/>
        </MarkerGroup></LDXFile>"#,
        )
        .unwrap();
        let a = ldx
            .specialty_analysis("rally".into(), 697.)
            .unwrap()
            .unwrap();
        let stages: Vec<_> = a
            .ranges
            .iter()
            .filter(|r| r.kind == crate::ranges::RangeKind::Stage)
            .collect();
        assert_eq!(stages.len(), 2);
        assert!((stages[0].start - 7.9848535).abs() < 1e-9);
        assert!((stages[1].end - 277.841856).abs() < 1e-9);
        assert_eq!(a.ranges.len(), 5);
        assert_eq!(a.corrections.speed_priority[0], "Ground Speed");
    }

    #[test]
    fn malformed_specialty_ranges_are_rejected_without_affecting_circuit() {
        let circuit = LdxFile::parse(
            r#"<LDXFile><Details><String Id="Total Laps" Value="1"/></Details></LDXFile>"#,
        )
        .unwrap();
        assert!(
            circuit
                .specialty_analysis("circuit".into(), 20.)
                .unwrap()
                .is_none()
        );
        let unpaired = LdxFile::parse(r#"<LDXFile><MarkerGroup Name="StageMarkers"><Marker ClassName="STGSTRT" Name="A" Time="1000000"/></MarkerGroup></LDXFile>"#).unwrap();
        assert!(unpaired.specialty_analysis("rally".into(), 20.).is_err());
        let outside = LdxFile::parse(r#"<LDXFile><MarkerGroup Name="Outings:Runs"><Marker ClassName="SOR" Time="21000000"/></MarkerGroup></LDXFile>"#).unwrap();
        assert!(outside.specialty_analysis("drag".into(), 20.).is_err());
    }
}
