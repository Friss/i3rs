//! Explicit, portable output contracts. Samples are selected on [start, end).
use crate::{Interpolation, channels::sample_with_policy};
use std::fmt::Write;

#[derive(Clone, Debug)]
pub struct OutputSeries {
    pub name: String,
    pub unit: String,
    pub data: Vec<f64>,
    pub rate: f64,
    pub precision: usize,
    pub interpolation: Interpolation,
    pub scale: f64,
    pub offset: f64,
}
impl OutputSeries {
    pub fn value(&self, t: f64) -> Option<f64> {
        // Do not extend a channel into the remainder of a longer recording.
        if t < 0. || t >= self.data.len() as f64 / self.rate {
            return None;
        }
        sample_with_policy(&self.data, self.rate, t, self.interpolation)
            .map(|v| v * self.scale + self.offset)
            .filter(|v| v.is_finite())
    }
}
#[derive(Clone, Copy, Debug)]
pub struct OutputPlan {
    pub start: f64,
    pub end: f64,
    pub rate: f64,
    pub relative_time: bool,
    pub include_time: bool,
}
impl OutputPlan {
    pub fn count(self) -> Result<usize, String> {
        if ![self.start, self.end, self.rate]
            .iter()
            .all(|x| x.is_finite())
            || self.start < 0.
            || self.end <= self.start
            || self.rate <= 0.
        {
            return Err("Choose finite bounds, start < end and a positive output rate".into());
        }
        let n = ((self.end - self.start) * self.rate).ceil().max(1.);
        if n > 10_000_000. {
            return Err(
                "Output exceeds 10 million rows; choose a lower rate or smaller range".into(),
            );
        }
        let mut count = n as usize;
        while count > 1 && self.time(count - 1) >= self.end {
            count -= 1;
        }
        Ok(count)
    }
    pub fn time(self, row: usize) -> f64 {
        self.start + row as f64 / self.rate
    }
}
pub fn csv_cell(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
/// CSV uses a selection-anchored fixed grid, per-channel interpolation and explicit units.
/// Distance is an optional independently sampled series in metres, relative to selection start.
pub fn series_csv(
    plan: OutputPlan,
    channels: &[OutputSeries],
    distance: Option<&OutputSeries>,
) -> Result<String, String> {
    if channels.is_empty() {
        return Err("Select at least one channel".into());
    }
    for c in channels.iter().chain(distance) {
        if !c.rate.is_finite()
            || c.rate <= 0.
            || c.precision > 15
            || !c.scale.is_finite()
            || !c.offset.is_finite()
        {
            return Err(format!("Invalid output metadata for {}", c.name));
        }
    }
    if distance.is_some_and(|d| d.unit != "m") {
        return Err("Distance output series must be in metres (m)".into());
    }
    let count = plan.count()?;
    if count.saturating_mul(channels.len() + 2) > 20_000_000 {
        return Err(
            "Output exceeds 20 million cells; select fewer channels or a smaller rate/scope".into(),
        );
    }
    let mut headers = Vec::new();
    if plan.include_time {
        headers.push("Time (s)".to_owned());
    }
    if distance.is_some() {
        headers.push("Distance (m)".to_owned());
    }
    for c in channels {
        headers.push(csv_cell(&if c.unit.is_empty() {
            c.name.clone()
        } else {
            format!("{} ({})", c.name, c.unit)
        }));
    }
    let mut out = headers.join(",");
    out.push('\n');
    let origin = distance.and_then(|d| d.value(plan.start));
    for i in 0..count {
        let t = plan.time(i);
        if t >= plan.end {
            break;
        }
        let mut first = true;
        if plan.include_time {
            write!(
                out,
                "{:.9}",
                if plan.relative_time {
                    t - plan.start
                } else {
                    t
                }
            )
            .unwrap();
            first = false;
        }
        if let Some(d) = distance {
            if !first {
                out.push(',');
            }
            first = false;
            if let Some(v) = d.value(t).zip(origin).map(|(v, o)| v - o) {
                write!(out, "{v:.6}").unwrap();
            }
        }
        for c in channels {
            if !first {
                out.push(',');
            }
            first = false;
            if let Some(v) = c.value(t) {
                write!(out, "{v:.prec$}", prec = c.precision).unwrap();
            }
        }
        out.push('\n');
    }
    Ok(out)
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
/// Google Earth KML. Invalid coordinates break the path instead of bridging outages.
pub fn earth_kml(
    name: &str,
    plan: OutputPlan,
    lat: &OutputSeries,
    lon: &OutputSeries,
) -> Result<String, String> {
    if [lat, lon].iter().any(|c| {
        !c.rate.is_finite() || c.rate <= 0. || !["deg", "°", ""].contains(&c.unit.as_str())
    }) {
        return Err("Earth coordinates need valid rates and degree units".into());
    }
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><kml xmlns=\"http://www.opengis.net/kml/2.2\"><Document><name>{}</name><Placemark><MultiGeometry>",
        xml(name)
    );
    let mut open = false;
    let mut first_point = None;
    let mut segments = 0;
    for i in 0..plan.count()? {
        let t = plan.time(i);
        if t >= plan.end {
            break;
        }
        let point = lat
            .value(t)
            .zip(lon.value(t))
            .filter(|(a, b)| a.abs() <= 90. && b.abs() <= 180. && (*a, *b) != (0., 0.));
        if let Some((a, b)) = point {
            if open {
                write!(out, "{b:.9},{a:.9},0 ").unwrap();
            } else if let Some((first_lat, first_lon)) = first_point.take() {
                out.push_str("<LineString><tessellate>1</tessellate><coordinates>");
                write!(out, "{first_lon:.9},{first_lat:.9},0 {b:.9},{a:.9},0 ").unwrap();
                open = true;
                segments += 1;
            } else {
                first_point = Some((a, b));
            }
        } else {
            if open {
                out.push_str("</coordinates></LineString>");
                open = false;
            }
            first_point = None;
        }
    }
    if open {
        out.push_str("</coordinates></LineString>");
    }
    out.push_str("</MultiGeometry></Placemark></Document></kml>");
    if segments == 0 {
        return Err(
            "Earth export requires at least two consecutive valid GPS positions in degrees".into(),
        );
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn series(interpolation: Interpolation) -> OutputSeries {
        OutputSeries {
            name: "State, \"test\"".into(),
            unit: "V".into(),
            data: vec![0., 10., 20.],
            rate: 1.,
            precision: 2,
            interpolation,
            scale: 2.,
            offset: 1.,
        }
    }
    #[test]
    fn off_grid_mixed_rate_never_anticipates_state_or_precedes_scope() {
        let p = OutputPlan {
            start: 0.25,
            end: 1.26,
            rate: 2.,
            relative_time: true,
            include_time: true,
        };
        let csv = series_csv(
            p,
            &[
                series(Interpolation::Linear),
                series(Interpolation::Previous),
            ],
            None,
        )
        .unwrap();
        assert_eq!(csv.lines().count(), 4);
        assert!(csv.contains("0.000000000,6.00,1.00"));
        assert!(csv.contains("0.500000000,16.00,1.00"));
        assert!(csv.contains("1.000000000,26.00,21.00"));
        assert!(csv.contains("State, \"\"test\"\" (V)"));
        assert!(
            series_csv(
                OutputPlan {
                    rate: f64::NAN,
                    ..p
                },
                &[series(Interpolation::Linear)],
                None
            )
            .is_err()
        );
    }
    #[test]
    fn missing_values_and_distance_keep_alignment() {
        let mut s = series(Interpolation::Previous);
        s.data[1] = f64::NAN;
        let p = OutputPlan {
            start: 0.5,
            end: 3.,
            rate: 2.,
            relative_time: false,
            include_time: true,
        };
        let mut distance = series(Interpolation::Linear);
        distance.unit = "m".into();
        let csv = series_csv(p, &[s], Some(&distance)).unwrap();
        assert!(csv.contains("0.500000000,0.000000,1.00"));
        assert!(csv.contains("1.000000000,10.000000,"));
        assert!(!csv.contains("NaN"));
    }
    #[test]
    fn earth_escapes_names_and_breaks_gps_outages() {
        let mut lat = series(Interpolation::Previous);
        lat.unit = "deg".into();
        lat.scale = 1.;
        lat.offset = 0.;
        lat.data = vec![
            35.,
            f64::NAN,
            36.,
            36.1,
            f64::NAN,
            36.2,
            36.3,
            f64::NAN,
            36.4,
        ];
        let mut lon = lat.clone();
        lon.data = vec![-78., -78., -79., -79.1, -79., -79.2, -79.3, -79., -79.4];
        let k = earth_kml(
            "A&B",
            OutputPlan {
                start: 0.,
                end: 9.,
                rate: 1.,
                relative_time: true,
                include_time: true,
            },
            &lat,
            &lon,
        )
        .unwrap();
        assert!(k.contains("A&amp;B"));
        assert_eq!(k.matches("<LineString>").count(), 2);
        let coordinates = k
            .split("<coordinates>")
            .skip(1)
            .map(|s| {
                s.split("</coordinates>")
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .count()
            })
            .collect::<Vec<_>>();
        assert_eq!(coordinates, vec![2, 2]);
        assert!(!k.contains("-78.000000000,35.000000000"));
        assert!(!k.contains("-79.400000000,36.400000000"));
    }
    #[test]
    fn earth_rejects_disconnected_singletons_instead_of_empty_geometries() {
        let mut lat = series(Interpolation::Previous);
        lat.unit = "deg".into();
        lat.scale = 1.;
        lat.offset = 0.;
        lat.data = vec![36., f64::NAN, 36.1];
        let mut lon = lat.clone();
        lon.data = vec![-79.; 3];
        assert!(
            earth_kml(
                "Singletons",
                OutputPlan {
                    start: 0.,
                    end: 3.,
                    rate: 1.,
                    relative_time: true,
                    include_time: true,
                },
                &lat,
                &lon
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    #[test]
    fn timestamp_can_be_omitted_without_empty_leading_cells() {
        let p = OutputPlan {
            start: 0.,
            end: 1.,
            rate: 1.,
            relative_time: true,
            include_time: false,
        };
        let s = OutputSeries {
            name: "Value".into(),
            unit: "V".into(),
            data: vec![2.],
            rate: 1.,
            precision: 1,
            interpolation: Interpolation::Previous,
            scale: 1.,
            offset: 0.,
        };
        assert_eq!(
            series_csv(p, std::slice::from_ref(&s), None).unwrap(),
            "\"Value (V)\"\n2.0\n"
        );
        let mut distance = s.clone();
        distance.unit = "m".into();
        assert_eq!(
            series_csv(p, std::slice::from_ref(&s), Some(&distance)).unwrap(),
            "Distance (m),\"Value (V)\"\n0.000000,2.0\n"
        );
    }
    #[test]
    fn arbitrarily_close_off_grid_end_still_includes_valid_grid_point() {
        let p = OutputPlan {
            start: 0.,
            end: 1. + 1e-10,
            rate: 1.,
            relative_time: true,
            include_time: true,
        };
        assert_eq!(p.count().unwrap(), 2);
        assert_eq!(OutputPlan { end: 1., ..p }.count().unwrap(), 1);
        assert!(OutputPlan { rate: 1e12, ..p }.count().is_err());
    }
}
