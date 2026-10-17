//! MoTeC TD1 v1 distance/time curves. The layout is verified against an i2
//! Standard 1.1.4.457 export; physical device acceptance is a separate concern.
use crate::{Session, output::OutputSeries};

const HEADER: usize = 2048;
const MAX_POINTS: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceLap {
    pub date: String,
    pub time: String,
    pub driver: String,
    pub vehicle: String,
    pub venue: String,
    /// Centiseconds since range start, paired with `distance_dm`.
    pub time_cs: Vec<u32>,
    /// Decimetres since range start. Strictly increasing, beginning at zero.
    pub distance_dm: Vec<u32>,
}

impl ReferenceLap {
    /// Save with the same staged publication/recovery policy as log output.
    pub fn write(&self, path: &std::path::Path) -> Result<Option<std::path::PathBuf>, String> {
        if !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("td1"))
        {
            return Err("Device reference filename must end in .td1".into());
        }
        crate::log_writer::publish_outputs(path, &[path.to_path_buf()], &[self.encode()?])
    }
    fn validate(&self) -> Result<(), String> {
        let n = self.time_cs.len();
        if !(2..=MAX_POINTS).contains(&n)
            || self.distance_dm.len() != n
            || self.time_cs[0] != 0
            || self.distance_dm[0] != 0
            || self.time_cs[n - 1] == 0
            || self.time_cs.windows(2).any(|p| p[1] < p[0])
            || self.distance_dm.windows(2).any(|p| p[1] <= p[0])
        {
            return Err(
                "Reference curve requires increasing distance and nondecreasing time".into(),
            );
        }
        Ok(())
    }

    /// Decode the observed v1 layout. Unknown versions/layouts are rejected.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let invalid = || "Unsupported or truncated TD1 reference lap".to_owned();
        if bytes.len() < HEADER || bytes[..2] != [1, 0] {
            return Err(invalid());
        }
        let u32_at = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let n = u32_at(6) as usize;
        if !(2..=MAX_POINTS).contains(&n)
            || u32_at(2) as usize != HEADER
            || u64::from_le_bytes(bytes[10..18].try_into().unwrap()) != HEADER as u64
            || u64::from_le_bytes(bytes[22..30].try_into().unwrap()) != (HEADER + n * 4) as u64
            || bytes.len() != HEADER + n * 8
            || bytes[426..430] != [1, 0, 0, 0]
        {
            return Err(invalid());
        }
        let string = |offset: usize, size: usize| -> Result<String, String> {
            let b = &bytes[offset..offset + size];
            let end = b.iter().position(|&c| c == 0).unwrap_or(size);
            String::from_utf8(b[..end].to_vec()).map_err(|_| "TD1 metadata is not UTF-8".into())
        };
        let lap = Self {
            date: string(40, 32)?,
            time: string(72, 32)?,
            driver: string(104, 64)?,
            vehicle: string(168, 128)?,
            venue: string(296, 128)?,
            time_cs: (0..n).map(|i| u32_at(HEADER + i * 4)).collect(),
            distance_dm: (0..n).map(|i| u32_at(HEADER + n * 4 + i * 4)).collect(),
        };
        lap.validate()?;
        if u32_at(18) != *lap.time_cs.last().unwrap()
            || u32_at(30) != *lap.distance_dm.last().unwrap()
        {
            return Err(invalid());
        }
        // Reserved/header variants must not be silently discarded by a rewrite.
        if lap.encode()?.as_slice() != bytes {
            return Err(invalid());
        }
        Ok(lap)
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let n = self.time_cs.len();
        let mut b = vec![0; HEADER];
        b[..2].copy_from_slice(&1u16.to_le_bytes());
        for (offset, value) in [
            (2, HEADER as u32),
            (6, n as u32),
            (18, *self.time_cs.last().unwrap()),
            (30, *self.distance_dm.last().unwrap()),
            (426, 1),
        ] {
            b[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        b[10..18].copy_from_slice(&(HEADER as u64).to_le_bytes());
        b[22..30].copy_from_slice(&((HEADER + n * 4) as u64).to_le_bytes());
        for (offset, size, s) in [
            (40, 32, &self.date),
            (72, 32, &self.time),
            (104, 64, &self.driver),
            (168, 128, &self.vehicle),
            (296, 128, &self.venue),
        ] {
            if s.len() >= size || s.contains('\0') || !s.is_ascii() {
                return Err(format!(
                    "TD1 metadata must be ASCII, without NUL, shorter than {size} bytes"
                ));
            }
            b[offset..offset + s.len()].copy_from_slice(s.as_bytes());
        }
        for v in self.time_cs.iter().chain(&self.distance_dm) {
            b.extend_from_slice(&v.to_le_bytes());
        }
        Ok(b)
    }

    /// Invert a finite, monotone corrected-distance series in metres. Output is
    /// a one-metre spatial grid plus the final decimetre-rounded endpoint, with
    /// linear time interpolation and nearest-centisecond quantization. Stops
    /// retain their elapsed time; the last point retains the exact range time.
    /// No user maths or display-unit transforms are applied here.
    pub fn from_distance(
        session: &Session,
        distance: &OutputSeries,
        start: f64,
        end: f64,
    ) -> Result<Self, String> {
        if distance.unit != "m"
            || !distance.rate.is_finite()
            || distance.rate <= 0.
            || !distance.scale.is_finite()
            || distance.scale <= 0.
            || !distance.offset.is_finite()
            || distance.interpolation != crate::Interpolation::Linear
            || !start.is_finite()
            || !end.is_finite()
            || start < 0.
            || end <= start
            || end > distance.data.len() as f64 / distance.rate
            || distance.data.len() > 20_000_000
        {
            return Err(
                "Reference lap requires a covered range and corrected distance in metres".into(),
            );
        }
        let origin = distance
            .value(start)
            .ok_or("Missing distance at range start")?;
        // The final sample covers its last sampling interval. Hold that value
        // at the exact recording boundary, without extrapolating past it.
        let endpoint = if end == distance.data.len() as f64 / distance.rate {
            distance.data.len().saturating_sub(1) as f64 / distance.rate
        } else {
            end
        };
        let last = distance
            .value(endpoint)
            .ok_or("Missing distance at range end")?
            - origin;
        let quantize = |v: f64| -> Result<u32, String> {
            if !v.is_finite() || v < 0. || v.round() > u32::MAX as f64 {
                Err("Reference curve exceeds TD1 numeric limits".into())
            } else {
                Ok(v.round() as u32)
            }
        };
        let final_dm = quantize(last * 10.)?;
        let final_cs = quantize((end - start) * 100.)?;
        if final_dm == 0 || final_cs == 0 || final_dm as usize / 10 + 2 > MAX_POINTS {
            return Err(
                "Reference range is too short or exceeds one million spatial points".into(),
            );
        }
        let mut samples = vec![(0., 0.)];
        let first = (start * distance.rate).floor() as usize + 1;
        let limit = (end * distance.rate).ceil() as usize;
        for i in first..limit {
            let t = i as f64 / distance.rate;
            let d = distance
                .value(t)
                .ok_or("Missing distance inside reference range")?
                - origin;
            if d + 1e-8 < samples.last().unwrap().1 {
                return Err("Reference distance must not run backwards".into());
            }
            samples.push((t - start, d));
        }
        if last + 1e-8 < samples.last().unwrap().1 {
            return Err("Reference distance must not run backwards".into());
        }
        samples.push((end - start, last));
        let mut lap = Self {
            date: session.date.clone(),
            time: session.time.clone(),
            driver: session.driver.clone(),
            vehicle: session.vehicle_id.clone(),
            venue: session.venue.clone(),
            time_cs: vec![0],
            distance_dm: vec![0],
        };
        let mut j = 1;
        for dm in (10..final_dm).step_by(10) {
            let target = dm as f64 / 10.;
            // Quantization can put the final rounded endpoint above the source.
            if target >= last {
                break;
            }
            while j + 1 < samples.len() && samples[j].1 < target {
                j += 1;
            }
            let (t0, d0) = samples[j - 1];
            let (t1, d1) = samples[j];
            if d1 <= d0 {
                return Err("Cannot invert stationary reference interval".into());
            }
            lap.time_cs.push(quantize(
                (t0 + (target - d0) / (d1 - d0) * (t1 - t0)) * 100.,
            )?);
            lap.distance_dm.push(dm);
        }
        lap.time_cs.push(final_cs);
        lap.distance_dm.push(final_dm);
        lap.validate()?;
        Ok(lap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Interpolation;
    #[test]
    fn i2_fixture_is_byte_exact() {
        let b = include_bytes!("../../../test_data/reference/i3r9/rally-stage-1.td1");
        let lap = ReferenceLap::decode(b).unwrap();
        assert_eq!(lap.driver, "Roger Outback");
        assert_eq!(lap.venue, "Belgrave");
        assert_eq!(lap.time_cs.len(), 3268);
        assert_eq!(lap.time_cs.last(), Some(&13400));
        assert_eq!(lap.distance_dm.last(), Some(&32670));
        assert_eq!(lap.encode().unwrap(), b);
        for i in [0, 2, 6, 10, 18, 22, 30, 426] {
            let mut bad = b.to_vec();
            bad[i] ^= 255;
            assert!(ReferenceLap::decode(&bad).is_err());
        }
        assert!(ReferenceLap::decode(&b[..b.len() - 1]).is_err());
    }
    fn series(data: Vec<f64>) -> OutputSeries {
        OutputSeries {
            name: "distance".into(),
            unit: "m".into(),
            data,
            rate: 2.,
            precision: 3,
            interpolation: Interpolation::Linear,
            scale: 1.,
            offset: 0.,
        }
    }
    #[test]
    fn off_grid_distance_inversion_and_stops() {
        let ld = crate::LdFile::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../test_data/VIR_LAP.ld"
        ))
        .unwrap();
        let d = series(vec![100., 101., 101., 103., 104., 105.25]);
        let l = ReferenceLap::from_distance(&ld.session, &d, 0.25, 2.5).unwrap();
        assert_eq!(l.distance_dm, vec![0, 10, 20, 30, 40, 48]);
        assert_eq!(l.time_cs, vec![0, 88, 113, 150, 195, 225]);
        assert_eq!(ReferenceLap::decode(&l.encode().unwrap()).unwrap(), l);
        let whole = ReferenceLap::from_distance(&ld.session, &d, 0., 3.).unwrap();
        assert_eq!(whole.time_cs.last(), Some(&300));
        assert_eq!(whole.distance_dm.last(), Some(&53));
        assert!(ReferenceLap::from_distance(&ld.session, &d, 0., 3.01).is_err());
        let mut bad = d.clone();
        bad.data[2] = f64::NAN;
        assert!(ReferenceLap::from_distance(&ld.session, &bad, 0., 2.5).is_err());
        bad.data[2] = 90.;
        assert!(ReferenceLap::from_distance(&ld.session, &bad, 0., 2.5).is_err());
    }
}
