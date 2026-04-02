//! Parser for MoTeC .ldx sidecar XML files.
//!
//! The .ldx file sits alongside the .ld file and contains lap timing
//! information written by MoTeC i2 or the ECU logging system.

use quick_xml::Reader;
use quick_xml::events::Event;
use std::path::Path;

/// Parsed lap timing data from a .ldx sidecar file.
#[derive(Debug, Clone, Default)]
pub struct LdxFile {
    pub total_laps: Option<u32>,
    pub fastest_time: Option<String>,
    pub fastest_lap: Option<u32>,
    /// Individual lap markers if present (start_time, end_time in seconds).
    pub laps: Vec<LdxLap>,
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

        loop {
            match reader.read_event() {
                Ok(Event::Empty(ref e)) if e.name().as_ref() == b"String" => {
                    let mut id = String::new();
                    let mut value = String::new();
                    for attr in e.attributes().flatten() {
                        match attr.key.as_ref() {
                            b"Id" => {
                                id = String::from_utf8_lossy(&attr.value).to_string();
                            }
                            b"Value" => {
                                value = String::from_utf8_lossy(&attr.value).to_string();
                            }
                            _ => {}
                        }
                    }
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

        Ok(ldx)
    }
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
}
