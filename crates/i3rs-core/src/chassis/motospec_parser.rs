//! Parser for MotoSPEC MS1/MS3 chassis definition files.
//!
//! MS1 and MS3 files use the same encoding: a simple shift cipher applied per character
//! position, producing a line-oriented plaintext with a structured key=value grammar.
//! Multi-setup MS3 files carry up to three setup columns (1, 2, 3); the caller selects
//! which column to load.

use std::collections::HashMap;
use std::path::Path;

use super::{
    AirSpringMode, CartridgeType, ChassisModel, ForkInstance, FrameInstance, LinkInstanceRecord,
    ShockInstance, SwingarmInstance, YokeInstance,
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Decode and parse a MotoSPEC MS1/MS3 file into a [`ChassisModel`].
///
/// `column` selects the setup column to load (1, 2, or 3).
/// Returns an error string if the file cannot be parsed or the column is absent.
pub fn parse_motospec_file(path: &Path, column: u8) -> Result<ChassisModel, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Cannot read chassis file: {e}"))?;
    let plaintext = decode_to_plaintext(&bytes);
    let parsed = parse_plaintext(&plaintext);
    validate(&parsed)?;

    let col_id = column.to_string();
    let available = list_column_ids(&parsed);

    if !available.contains(&col_id) {
        return Err(format!(
            "Column '{}' not found. Available: {}",
            col_id,
            available.join(", ")
        ));
    }

    merge_sibling_hardware(&mut parsed.clone(), &col_id);
    let col = parsed.columns.get(&col_id)
        .ok_or_else(|| format!("Column '{col_id}' missing after merge"))?;

    Ok(build_model(col, &parsed.columns, &col_id))
}

/// Return the setup column IDs present in a MotoSPEC file (typically ["1"], ["1","2","3"]).
pub fn detect_columns(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Cannot read chassis file: {e}"))?;
    let plaintext = decode_to_plaintext(&bytes);
    let parsed = parse_plaintext(&plaintext);
    Ok(list_column_ids(&parsed)
        .into_iter()
        .filter_map(|s| s.parse::<u8>().ok())
        .collect())
}

// ---------------------------------------------------------------------------
// Internal structures
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct ColumnState {
    exporting_version: String,
    comments_line: String,
    notes: String,
    scalars: HashMap<String, String>,
    /// component_type → list of instance property maps
    components: HashMap<String, Vec<HashMap<String, String>>>,
}

#[derive(Debug, Default, Clone)]
struct ParsedFile {
    file_header: HashMap<String, String>,
    columns: HashMap<String, ColumnState>,
}

const NOTES_SENTINEL: &str = "'EnD oF nOtEs";

const COMPONENT_TYPES: &[&str] = &[
    "Frame", "Yoke", "Swingarm", "Clevis",
    "Fork", "Shock", "FrSpring", "RrSpring",
    "FrTire", "RrTire", "Link", "MeasRHG",
];

const CHASSIS_SIGNALS: &[&str] = &["WhlCtrHypotenuse", "LinkType", "SwL", "EffSwL"];

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

fn decode_to_plaintext(raw: &[u8]) -> String {
    let trimmed = trim_bom_and_newlines(raw);
    let encoded = String::from_utf8_lossy(trimmed).into_owned();
    decode_shift(&encoded)
}

fn trim_bom_and_newlines(raw: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = raw.len();
    // Strip UTF-8 BOM
    if end >= 3 && raw[0] == 0xEF && raw[1] == 0xBB && raw[2] == 0xBF {
        start += 3;
    }
    while end > start && matches!(raw[end - 1], b'\r' | b'\n') {
        end -= 1;
    }
    &raw[start..end]
}

fn decode_shift(encoded: &str) -> String {
    let mut result = String::with_capacity(encoded.len());
    let mut pos: u32 = 0;
    for ch in encoded.chars() {
        pos += 1;
        let code = ch as u32;
        let shifted = if pos % 2 == 0 { code + 1 } else { code + 2 };
        if let Some(decoded) = char::from_u32(shifted) {
            result.push(decoded);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Parse plaintext
// ---------------------------------------------------------------------------

fn parse_plaintext(plaintext: &str) -> ParsedFile {
    let lines: Vec<&str> = plaintext.split("\r\n").collect();
    let mut file_header: HashMap<String, String> = HashMap::new();
    let mut columns: HashMap<String, ColumnState> = HashMap::new();

    let mut i = 0;

    // File header lines starting with "__"
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("__") {
            let eq = line.find('=');
            let key = eq.map(|p| &line[..p]).unwrap_or(line).trim_start_matches('_');
            let val = eq.map(|p| &line[p + 1..]).unwrap_or("");
            file_header.insert(key.to_string(), val.to_string());
            i += 1;
        } else if line.is_empty() {
            i += 1;
        } else {
            break;
        }
    }

    let mut in_notes = false;
    let mut notes_col: Option<String> = None;
    let mut notes_buf: Vec<String> = Vec::new();

    while i < lines.len() {
        let line = lines[i];

        if in_notes {
            if let Some(pos) = line.find(NOTES_SENTINEL) {
                notes_buf.push(line[..pos].to_string());
                if let Some(ref col) = notes_col {
                    ensure_column(&mut columns, col).notes = notes_buf.join("\r\n");
                }
                in_notes = false;
                notes_buf.clear();
            } else {
                notes_buf.push(line.to_string());
            }
            i += 1;
            continue;
        }

        if line.is_empty() {
            i += 1;
            continue;
        }

        // Try component line: {1|2|3}_{TypeName}{idx}|{SubKey}={value}
        if let Some(comp) = try_parse_component(line) {
            let col = ensure_column(&mut columns, &comp.0);
            let list = col.components.entry(comp.1.clone()).or_default();
            while list.len() <= comp.2 {
                list.push(HashMap::new());
            }
            list[comp.2].insert(comp.3, comp.4);
            i += 1;
            continue;
        }

        // Try scalar line: {1|2|3|_}_{key}={value}
        if let Some((col_id, key, value)) = try_parse_scalar(line) {
            if col_id == "_" {
                // Broadcast to all present columns (or create them)
                for cid in ["1", "2", "3"] {
                    if columns.contains_key(cid) {
                        apply_scalar(&mut columns, cid, &key, &value);
                    }
                }
            } else {
                if key == "Notes" {
                    in_notes = true;
                    notes_col = Some(col_id.clone());
                    notes_buf = vec![value.clone()];
                } else {
                    apply_scalar(&mut columns, &col_id, &key, &value);
                }
            }
            i += 1;
            continue;
        }

        i += 1;
    }

    compact_empty_components(&mut columns);
    ParsedFile { file_header, columns }
}

fn apply_scalar(columns: &mut HashMap<String, ColumnState>, col_id: &str, key: &str, value: &str) {
    let col = ensure_column(columns, col_id);
    if key == "ExportingMotoSPECversion" {
        col.exporting_version = value.to_string();
    } else if key == "Comments" {
        col.comments_line = value.to_string();
    } else {
        col.scalars.insert(key.to_string(), value.to_string());
    }
}

/// Returns (col_id, type_name, instance_idx, subkey, value) for a component line.
fn try_parse_component(line: &str) -> Option<(String, String, usize, String, String)> {
    // Pattern: {col}_{Type}{idx}|{SubKey}={value}
    let (col, rest) = line.split_once('_')?;
    if !matches!(col, "1" | "2" | "3") {
        return None;
    }
    let (type_idx, subkv) = rest.split_once('|')?;
    let (subkey, value) = subkv.split_once('=')?;

    // type_idx = "Frame0", "Fork1", etc.
    let type_name = COMPONENT_TYPES.iter().find(|&&t| type_idx.starts_with(t))?;
    let idx_str = &type_idx[type_name.len()..];
    let idx: usize = idx_str.parse().ok()?;

    Some((
        col.to_string(),
        type_name.to_string(),
        idx,
        subkey.to_string(),
        value.to_string(),
    ))
}

/// Returns (col_id, key, value) for a scalar line.
fn try_parse_scalar(line: &str) -> Option<(String, String, String)> {
    let (col, rest) = line.split_once('_')?;
    if !matches!(col, "1" | "2" | "3" | "_") {
        return None;
    }
    let (key, value) = rest.split_once('=')?;
    Some((col.to_string(), key.to_string(), value.to_string()))
}

fn ensure_column<'a>(columns: &'a mut HashMap<String, ColumnState>, id: &str) -> &'a mut ColumnState {
    columns.entry(id.to_string()).or_default()
}

fn compact_empty_components(columns: &mut HashMap<String, ColumnState>) {
    for col in columns.values_mut() {
        col.components.retain(|_, v| !v.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Column enumeration and validation
// ---------------------------------------------------------------------------

fn list_column_ids(parsed: &ParsedFile) -> Vec<String> {
    let mut ids: Vec<String> = parsed.columns.iter()
        .filter(|(_, v)| column_has_payload(v))
        .map(|(k, _)| k.clone())
        .collect();
    ids.sort();
    ids
}

fn column_has_payload(col: &ColumnState) -> bool {
    !col.exporting_version.is_empty()
        || !col.comments_line.is_empty()
        || !col.notes.is_empty()
        || !col.scalars.is_empty()
        || col.components.values().any(|v| !v.is_empty())
}

fn validate(parsed: &ParsedFile) -> Result<(), String> {
    if parsed.file_header.is_empty() && list_column_ids(parsed).is_empty() {
        return Err("File does not appear to be a MotoSPEC chassis file: no header or column data.".into());
    }
    if !has_chassis_marker(parsed) {
        return Err("File does not appear to be a MotoSPEC chassis file: expected chassis markers.".into());
    }
    Ok(())
}

fn has_chassis_marker(parsed: &ParsedFile) -> bool {
    for col in parsed.columns.values() {
        if !col.exporting_version.is_empty() { return true; }
        for sig in CHASSIS_SIGNALS {
            if col.scalars.contains_key(*sig) { return true; }
        }
        if col.components.values().any(|v| !v.is_empty()) { return true; }
    }
    for v in parsed.file_header.values() {
        if v.to_ascii_uppercase().contains("MOTOSPEC") { return true; }
    }
    false
}

// ---------------------------------------------------------------------------
// Sibling hardware merge (MS3 multi-setup files)
// ---------------------------------------------------------------------------

fn merge_sibling_hardware(parsed: &mut ParsedFile, primary_col_id: &str) {
    let sibling_ids: Vec<String> = {
        let mut ids: Vec<String> = parsed.columns.keys().cloned().collect();
        ids.sort();
        ids
    };

    for &comp_type in COMPONENT_TYPES {
        let primary_has = parsed.columns.get(primary_col_id)
            .map(|c| component_instances_meaningful(c, comp_type))
            .unwrap_or(false);
        if primary_has { continue; }

        let donor = sibling_ids.iter()
            .filter(|id| id.as_str() != primary_col_id)
            .find_map(|sid| {
                parsed.columns.get(sid)
                    .filter(|c| component_instances_meaningful(c, comp_type))
                    .and_then(|c| c.components.get(comp_type).cloned())
            });

        if let Some(instances) = donor {
            if let Some(primary) = parsed.columns.get_mut(primary_col_id) {
                primary.components.insert(comp_type.to_string(), instances);
            }
        }
    }
}

fn component_instances_meaningful(col: &ColumnState, comp_type: &str) -> bool {
    col.components.get(comp_type)
        .map(|v| !v.is_empty() && v.iter().any(|d| !d.is_empty()))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// ChassisModel construction
// ---------------------------------------------------------------------------

fn build_model(
    col: &ColumnState,
    all_columns: &HashMap<String, ColumnState>,
    col_id: &str,
) -> ChassisModel {
    // Merge: use this column's data, but also look at the merged result
    // (merge_sibling_hardware has already been applied to parsed, so col is fully merged)
    let _ = all_columns;
    let _ = col_id;

    let s = &col.scalars;

    let whl = parse_f64(s.get("WhlCtrHypotenuse")).unwrap_or(1480.0);
    let sw_l = parse_f64(s.get("SwL"))
        .or_else(|| parse_f64(s.get("EffSwL")))
        .unwrap_or(0.0);
    let link_type = s.get("LinkType").cloned();
    let fr_sprocket = parse_i32(s.get("FrSprocket")).unwrap_or(16);
    let rr_sprocket = parse_i32(s.get("RrSprocket")).unwrap_or(48);
    let chain_pitch_raw = s.get("ChainPitch").cloned();
    let chain_pitch_mm = resolve_chain_pitch_mm(chain_pitch_raw.as_deref());
    let sel_fr_tire = parse_i32(s.get("SelectedFrTireIndex")).unwrap_or(0);
    let sel_rr_tire = parse_i32(s.get("SelectedRrTireIndex")).unwrap_or(0);
    let sel_link = parse_i32(s.get("SelectedLinkIndex")).unwrap_or(0);
    let sel_fork = parse_i32(s.get("SelectedForkIndex")).unwrap_or(0);
    let sel_yoke = parse_i32(s.get("SelectedYokeIndex")).unwrap_or(0);
    let fork_pos = parse_f64(s.get("ForkPos")).unwrap_or(0.0);
    let yoke_offset = parse_f64(s.get("YokeOffset")).unwrap_or(0.0);
    let pivot_x = parse_f64(s.get("PivotX")).unwrap_or(0.0);
    let pivot_y = parse_f64(s.get("PivotY")).unwrap_or(0.0);
    let cof_g_h = parse_f64(s.get("CofGH")).unwrap_or(0.0);
    let cof_g_v = parse_f64(s.get("CofGV")).unwrap_or(0.0);
    let data_cof_g_x = parse_f64(s.get("DataCofGX")).unwrap_or(0.0);
    let hd_adj = s.get("HdAdj").cloned();
    let fork_ht_ref = s.get("ForkHtRef").cloned();
    let upr_hd_adj = parse_f64(s.get("UprHdAdj")).unwrap_or(0.0);
    let lwr_hd_adj = parse_f64(s.get("LwrHdAdj")).unwrap_or(0.0);
    let dual_shock = parse_bool(s.get("boolDualRrShock"));
    let shock_l_ext = parse_f64(s.get("ShockLExt")).unwrap_or(0.0);
    let spring_rate = parse_f64(s.get("SpringRate")).unwrap_or(0.0);
    let preload = parse_f64(s.get("Preload")).unwrap_or(0.0);
    let topout_l = parse_f64(s.get("TopoutL")).unwrap_or(0.0);
    let topout_rate = parse_f64(s.get("TopoutRate")).unwrap_or(0.0);
    let fork_l_spr_rate = parse_f64(s.get("ForkLSprRate")).unwrap_or(0.0);
    let fork_r_spr_rate = parse_f64(s.get("ForkRSprRate")).unwrap_or(0.0);
    let fork_l_spr_pre_l = parse_f64(s.get("ForkLSprPreL")).unwrap_or(0.0);
    let fork_r_spr_pre_l = parse_f64(s.get("ForkRSprPreL")).unwrap_or(0.0);
    let fork_l_oil_level = parse_f64(s.get("ForkLOilLevel")).unwrap_or(0.0);
    let fork_r_oil_level = parse_f64(s.get("ForkROilLevel")).unwrap_or(0.0);
    let fork_l_top_rate = parse_f64(s.get("ForkLTopRate")).unwrap_or(0.0);
    let fork_r_top_rate = parse_f64(s.get("ForkRTopRate")).unwrap_or(0.0);
    let fork_l_top_l = parse_f64(s.get("ForkLTopL")).unwrap_or(0.0);
    let fork_r_top_l = parse_f64(s.get("ForkRTopL")).unwrap_or(0.0);
    let ride_ht_ref = s.get("RideHtRef").cloned();
    let design_axle_below_pivot = if is_vertical_pivot_axle_ref(ride_ht_ref.as_deref()) {
        parse_f64(s.get("RideHtPtV")).filter(|&v| v < -10.0 && v > -400.0)
    } else {
        None
    };

    let (fr_tire_rad, fr_major, fr_minor) = try_tire_profile(&col.components, "FrTire", "FrTireInstance", sel_fr_tire);
    let (rr_tire_rad, rr_major, rr_minor) = try_tire_profile(&col.components, "RrTire", "RrTireInstance", sel_rr_tire);
    let front_tire_rad = fr_tire_rad.filter(|&r| r > 0.0).unwrap_or(whl * 0.2);
    let rear_tire_rad = rr_tire_rad.filter(|&r| r > 0.0).unwrap_or(whl * 0.22);

    // Frame
    let frame = col.components.get("Frame")
        .and_then(|v| v.first())
        .map(|props| FrameInstance {
            head_angle_deg: pf(props, "HeadAngle"),
            head_x: pf(props, "HeadX"),
            head_y: pf(props, "HeadY"),
            head_ht: pf(props, "HeadHt"),
            link_mnt_x: pf(props, "LinkMntX"),
            link_mnt_y: pf(props, "LinkMntY"),
            shock_mnt_x: pf(props, "ShockMntX"),
            shock_mnt_y: pf(props, "ShockMntY"),
            c_shaft_x: pf(props, "CShaftX"),
            c_shaft_y: pf(props, "CShaftY"),
        });

    // Swingarm
    let swingarm = col.components.get("Swingarm")
        .and_then(|v| v.first())
        .map(|props| SwingarmInstance {
            offset: pf(props, "Offset"),
            link_x: pf(props, "LinkX"),
            link_y: pf(props, "LinkY"),
            shock_x: pf(props, "ShockX"),
            shock_y: pf(props, "ShockY"),
            ecc_radius: pf(props, "EccRadius"),
        });

    // Fork (select by index)
    let fork = col.components.get("Fork")
        .and_then(|v| v.get(sel_fork as usize).or_else(|| v.first()))
        .map(|props| {
            let (oil_levels, air_volumes) = parse_oil_air_table(props);
            ForkInstance {
                length_mm: pf(props, "L"),
                upr_tube_l_mm: pf(props, "UprTubeL"),
                travel_mm: pf(props, "Travel"),
                lwr_offset_mm: pf(props, "LwrOffset"),
                fork_l_spr_rate_n_per_mm: fork_l_spr_rate,
                fork_r_spr_rate_n_per_mm: fork_r_spr_rate,
                fork_l_spr_pre_l_mm: fork_l_spr_pre_l,
                fork_r_spr_pre_l_mm: fork_r_spr_pre_l,
                fork_l_oil_level_mm: fork_l_oil_level,
                fork_r_oil_level_mm: fork_r_oil_level,
                fork_l_top_rate_n_per_mm: fork_l_top_rate,
                fork_r_top_rate_n_per_mm: fork_r_top_rate,
                fork_l_top_l_mm: fork_l_top_l,
                fork_r_top_l_mm: fork_r_top_l,
                bump_rate_n_per_mm: pf(props, "BumpRate"),
                bump_l_mm: pf(props, "BumpL"),
                air_spring_mode: parse_air_spring_mode(props.get("AirSpring").map(String::as_str)),
                tube_dia_mm: pf(props, "TubeDia"),
                rod_dia_mm: pf(props, "RodDia"),
                p_bar: parse_f64(props.get("P")).unwrap_or(1.0),
                kappa: parse_f64(props.get("Kappa")).unwrap_or(1.4),
                oil_levels,
                air_volumes,
                cartridge_type: parse_cartridge_type(props.get("Cartridge").map(String::as_str)),
                rod_thru_res_piston: parse_bool(props.get("RodThruResPiston")),
                asym_res: parse_bool(props.get("AsymRes")),
                res_pressure_bar: pf(props, "ResP"),
                res_piston_dia_mm: pf(props, "ResPistonDia"),
                res_piston_dia_inner_mm: pf(props, "ResPistonDiaInner"),
                res_spring_rate_n_per_mm: pf(props, "ResSpringRate"),
                res_spring_preload_mm: pf(props, "ResSpringPreload"),
                res_spring_chamber_vol_cc: pf(props, "ResSpringChamberVol"),
            }
        });

    // Shock
    let shock = col.components.get("Shock")
        .and_then(|v| v.first())
        .map(|props| ShockInstance {
            length_extended_mm: shock_l_ext,
            spring_rate_n_per_mm: spring_rate,
            stroke_mm: parse_f64(props.get("Stroke")).unwrap_or(75.0),
            shock_type: props.get("Shock").cloned(),
            res_pressure_bar: pf(props, "ResP"),
            res_vol_cc: pf(props, "ResVol"),
            rod_dia_mm: pf(props, "RodDia"),
            bump_ht_mm: pf(props, "BumpHt"),
            bump_rate_n_per_mm: pf(props, "BumpRate"),
        });

    // Link (select by index)
    let link = col.components.get("Link")
        .and_then(|v| {
            v.iter().enumerate().find(|(i, _)| *i == sel_link as usize)
                .or_else(|| v.first().map(|p| (0, p)))
                .map(|(_, p)| p)
        })
        .map(|props| LinkInstanceRecord {
            name: props.get("Name").cloned(),
            anchor_shock: pf(props, "AnchorShock"),
            anchor_linkarm: pf(props, "AnchorLinkarm"),
            shock_linkarm: pf(props, "ShockLinkarm"),
            nom_linkarm_l: pf(props, "NomLinkarmL"),
            rocker_orientation: props.get("RockerOrientation").cloned(),
            nom_shock_l: pf(props, "NomShockL"),
        });

    // Yoke (select by index)
    let yoke = col.components.get("Yoke")
        .and_then(|v| v.get(sel_yoke as usize).or_else(|| v.first()))
        .map(|props| YokeInstance {
            upr_yoke_ht: pf(props, "UprYokeHt"),
            lwr_yoke_ht: pf(props, "LwrYokeHt"),
        });

    ChassisModel {
        comments: if col.comments_line.is_empty() { None } else { Some(col.comments_line.clone()) },
        motospec_version: if col.exporting_version.is_empty() { None } else { Some(col.exporting_version.clone()) },
        link_type,
        wheel_center_hypotenuse_mm: whl,
        fr_sprocket_teeth: fr_sprocket,
        rr_sprocket_teeth: rr_sprocket,
        chain_pitch_raw,
        chain_pitch_mm,
        selected_fr_tire_index: sel_fr_tire,
        selected_rr_tire_index: sel_rr_tire,
        selected_link_index: sel_link,
        selected_fork_index: sel_fork,
        selected_yoke_index: sel_yoke,
        fork_pos_mm: fork_pos,
        front_tire_rad_mm: front_tire_rad,
        rear_tire_rad_mm: rear_tire_rad,
        fr_tire_major_rad_mm: fr_major.unwrap_or(0.0),
        fr_tire_minor_rad_mm: fr_minor.unwrap_or(0.0),
        rr_tire_major_rad_mm: rr_major.unwrap_or(0.0),
        rr_tire_minor_rad_mm: rr_minor.unwrap_or(0.0),
        sw_l_mm: sw_l,
        ride_ht_ref,
        yoke_offset_mm: yoke_offset,
        design_axle_below_pivot_mm: design_axle_below_pivot,
        hd_adj,
        fork_ht_ref,
        upr_hd_adj_mm: upr_hd_adj,
        lwr_hd_adj_mm: lwr_hd_adj,
        cof_g_h,
        cof_g_v,
        data_cof_g_x,
        pivot_x_mm: pivot_x,
        pivot_y_mm: pivot_y,
        shock_preload_mm: preload,
        topout_l_mm: topout_l,
        topout_rate_n_per_mm: topout_rate,
        dual_rr_shock: dual_shock,
        frame,
        swingarm,
        fork,
        shock,
        link,
        yoke,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn pf(props: &HashMap<String, String>, key: &str) -> f64 {
    parse_f64(props.get(key)).unwrap_or(0.0)
}

fn parse_f64(s: Option<&String>) -> Option<f64> {
    s.and_then(|v| v.trim().parse::<f64>().ok())
}

fn parse_i32(s: Option<&String>) -> Option<i32> {
    s.and_then(|v| v.trim().parse::<i32>().ok())
}

fn parse_bool(s: Option<&String>) -> bool {
    s.map(|v| v.trim().eq_ignore_ascii_case("true")).unwrap_or(false)
}

fn resolve_chain_pitch_mm(raw: Option<&str>) -> f64 {
    let Some(raw) = raw else { return 15.875 };
    if let Ok(mm) = raw.trim().parse::<f64>() {
        if mm > 1.0 && mm < 50.0 { return mm; }
    }
    match raw.trim().to_ascii_uppercase().as_str() {
        "FOUR" | "428" => 12.7,
        "FIVE" | "520" | "525" | "530" => 15.875,
        "SIX" | "630" => 19.05,
        "EIGHT" | "EIGHTH" => 25.4,
        _ => 15.875,
    }
}

fn is_vertical_pivot_axle_ref(ref_type: Option<&str>) -> bool {
    matches!(ref_type.map(|r| r.to_ascii_uppercase()).as_deref(),
        Some("VERTICAL_PIVOT-AXLE") | Some("VERTICAL_WHEEL_POSITION"))
}

fn parse_air_spring_mode(raw: Option<&str>) -> AirSpringMode {
    match raw.map(|r| r.trim().to_ascii_uppercase()).as_deref() {
        Some("NOMINAL_OIL_LEVEL") => AirSpringMode::NominalOilLevel,
        Some("OIL_LEVEL_TABLE") => AirSpringMode::OilLevelTable,
        Some("FORK_VOLUME") => AirSpringMode::ForkVolume,
        _ => AirSpringMode::Unknown,
    }
}

fn parse_cartridge_type(raw: Option<&str>) -> CartridgeType {
    match raw.map(|r| r.trim().to_ascii_uppercase()).as_deref() {
        Some("GAS") => CartridgeType::Gas,
        Some("GAS_AND_SPRING") => CartridgeType::GasAndSpring,
        Some("SPRING") => CartridgeType::Spring,
        Some("SPRING_MECH") => CartridgeType::SpringMech,
        Some("THRU_ROD") => CartridgeType::ThruRod,
        _ => CartridgeType::Unknown,
    }
}

fn parse_oil_air_table(props: &HashMap<String, String>) -> (Vec<f64>, Vec<f64>) {
    let mut oils = Vec::new();
    let mut airs = Vec::new();
    for i in 1..=50 {
        let oil = parse_f64(props.get(&format!("Oil{i}")));
        let air = parse_f64(props.get(&format!("Air{i}")));
        if let (Some(o), Some(a)) = (oil, air) {
            if o > 0.0 && a > 0.0 {
                oils.push(o);
                airs.push(a);
            }
        }
    }
    (oils, airs)
}

fn try_tire_profile(
    components: &HashMap<String, Vec<HashMap<String, String>>>,
    group: &str,
    _instance_name: &str,
    index: i32,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    let inst = components.get(group)
        .and_then(|v| v.get(index as usize).or_else(|| v.first()));
    match inst {
        Some(props) => (
            parse_f64(props.get("Rad")),
            parse_f64(props.get("MajorRad")),
            parse_f64(props.get("MinorRad")),
        ),
        None => (None, None, None),
    }
}
