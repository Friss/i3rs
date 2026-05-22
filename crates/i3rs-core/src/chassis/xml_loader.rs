//! MotoSPEC XML chassis loader — parses MotoSPEC XML files into [`ChassisModel`].
//!
//! MotoSPEC can export a chassis definition as an XML file carrying the same
//! data as an MS1/MS3 binary file in a human-readable form.  This loader parses
//! those files using the same field interpretation rules as the binary decoder,
//! and is a direct port of the C# `XmlChassisLoader`.
//!
//! # File structure (abbreviated)
//! ```xml
//! <MotoSPEC>
//!   <Chassis column="1">
//!     <Settings> ... flat key/value elements ... </Settings>
//!     <Components>
//!       <Frame><FrameInstance index="0"> ... </FrameInstance></Frame>
//!       <Swingarm><SwingarmInstance index="0"> ... </SwingarmInstance></Swingarm>
//!       <Fork><ForkInstance index="0|1|…"> ... </ForkInstance></Fork>
//!       <Shock><ShockInstance index="0"> ... </ShockInstance></Shock>
//!       <Link><LinkInstance index="0|1|…"> ... </LinkInstance></Link>
//!       <Yoke><YokeInstance index="0"> ... </YokeInstance></Yoke>
//!       <FrTire><FrTireInstance index="N"> ... </FrTireInstance></FrTire>
//!       <RrTire><RrTireInstance index="N"> ... </RrTireInstance></RrTire>
//!     </Components>
//!   </Chassis>
//! </MotoSPEC>
//! ```

use std::collections::HashMap;
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::chassis::{
    AirSpringMode, CartridgeType, ChassisModel, ForkInstance, FrameInstance, LinkInstanceRecord,
    ShockInstance, SwingarmInstance, YokeInstance,
};

/// Parse a MotoSPEC XML chassis file into a [`ChassisModel`].
pub fn parse_chassis_xml(path: &Path) -> Result<ChassisModel, String> {
    let xml = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read XML chassis file: {e}"))?;
    parse_chassis_xml_str(&xml)
}

/// Parse MotoSPEC XML content from a string slice into a [`ChassisModel`].
pub fn parse_chassis_xml_str(xml: &str) -> Result<ChassisModel, String> {
    let doc = XmlDoc::parse(xml)?;
    build_model(&doc)
}

// ---------------------------------------------------------------------------
// Minimal DOM representation
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct XmlNode {
    name: String,
    attrs: HashMap<String, String>,
    text: String,
    children: Vec<XmlNode>,
}

impl XmlNode {
    fn child(&self, name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|c| c.name == name)
    }

    fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.trim())
    }

    fn child_f64(&self, name: &str) -> Option<f64> {
        self.child_text(name)?.parse::<f64>().ok()
    }

    fn child_f64_or(&self, name: &str, default: f64) -> f64 {
        self.child_f64(name).unwrap_or(default)
    }

    fn child_i32_or(&self, name: &str, default: i32) -> i32 {
        self.child_text(name)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(default)
    }

    fn child_bool(&self, name: &str) -> bool {
        matches!(self.child_text(name), Some("True") | Some("true"))
    }

    /// Find the first child element with `name` whose `index` attribute equals `idx`.
    fn child_by_index(&self, name: &str, idx: i32) -> Option<&XmlNode> {
        self.children.iter().find(|c| {
            c.name == name
                && c.attrs
                    .get("index")
                    .and_then(|v| v.parse::<i32>().ok())
                    .map_or(false, |i| i == idx)
        })
    }
}

struct XmlDoc {
    root: XmlNode,
}

impl XmlDoc {
    fn parse(xml: &str) -> Result<Self, String> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut stack: Vec<XmlNode> = vec![XmlNode {
            name: "__root__".into(),
            ..Default::default()
        }];

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let name = std::str::from_utf8(e.local_name().into_inner())
                        .unwrap_or("")
                        .to_owned();
                    let mut attrs = HashMap::new();
                    for attr in e.attributes().flatten() {
                        if let (Ok(k), Ok(v)) = (
                            std::str::from_utf8(attr.key.local_name().into_inner()),
                            std::str::from_utf8(&attr.value),
                        ) {
                            attrs.insert(k.to_owned(), v.to_owned());
                        }
                    }
                    stack.push(XmlNode { name, attrs, ..Default::default() });
                }
                Ok(Event::End(_)) => {
                    if stack.len() > 1 {
                        let node = stack.pop().unwrap();
                        stack.last_mut().unwrap().children.push(node);
                    }
                }
                Ok(Event::Text(e)) => {
                    let raw = String::from_utf8_lossy(e.as_ref());
                    let trimmed = raw.trim().to_owned();
                    if !trimmed.is_empty() {
                        if let Some(top) = stack.last_mut() {
                            top.text = trimmed;
                        }
                    }
                }
                Ok(Event::Empty(e)) => {
                    let name = std::str::from_utf8(e.local_name().into_inner())
                        .unwrap_or("")
                        .to_owned();
                    let mut attrs = HashMap::new();
                    for attr in e.attributes().flatten() {
                        if let (Ok(k), Ok(v)) = (
                            std::str::from_utf8(attr.key.local_name().into_inner()),
                            std::str::from_utf8(&attr.value),
                        ) {
                            attrs.insert(k.to_owned(), v.to_owned());
                        }
                    }
                    stack.last_mut().unwrap().children.push(XmlNode {
                        name,
                        attrs,
                        ..Default::default()
                    });
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(format!("XML parse error: {e}")),
                _ => {}
            }
        }

        let mut root_wrapper = stack.pop().unwrap();
        // The actual document root is the first (and only) child of the sentinel.
        let root = root_wrapper.children.pop().unwrap_or(root_wrapper);
        Ok(XmlDoc { root })
    }
}

// ---------------------------------------------------------------------------
// Model building — mirrors XmlChassisLoader.cs field-for-field
// ---------------------------------------------------------------------------

fn build_model(doc: &XmlDoc) -> Result<ChassisModel, String> {
    // Root is either <MotoSPEC> (containing <Chassis>) or <Chassis> directly.
    let chassis = if doc.root.name == "Chassis" {
        &doc.root
    } else {
        doc.root.child("Chassis").ok_or("No <Chassis> element found")?
    };

    let settings = chassis.child("Settings").ok_or("No <Settings> in chassis XML")?;
    let comments  = chassis.child_text("Comments").map(str::to_owned);
    let version   = chassis.child_text("MotoSPECVersion").map(str::to_owned);
    let components = chassis.child("Components");

    // Settings — scalars
    let whl          = settings.child_f64_or("WhlCtrHypotenuse", 1480.0);
    let sw_l         = settings.child_f64("SwL")
                               .or_else(|| settings.child_f64("EffSwL"))
                               .unwrap_or(0.0);
    let link_type    = settings.child_text("LinkType").map(str::to_owned);
    let fr_sprocket  = settings.child_i32_or("FrSprocket", 16);
    let rr_sprocket  = settings.child_i32_or("RrSprocket", 48);
    let chain_pitch_raw = settings.child_text("ChainPitch").map(str::to_owned);
    let chain_pitch_mm  = resolve_chain_pitch_mm(chain_pitch_raw.as_deref());

    let sel_fr_tire  = settings.child_i32_or("SelectedFrTireIndex", 0);
    let sel_rr_tire  = settings.child_i32_or("SelectedRrTireIndex", 0);
    let sel_link     = settings.child_i32_or("SelectedLinkIndex", 0);
    let sel_fork     = settings.child_i32_or("SelectedForkIndex", 0);
    let sel_yoke     = settings.child_i32_or("SelectedYokeIndex", 0);

    let fork_pos      = settings.child_f64_or("ForkPos",    0.0);
    let yoke_offset   = settings.child_f64_or("YokeOffset", 0.0);
    let pivot_x       = settings.child_f64_or("PivotX",     0.0);
    let pivot_y       = settings.child_f64_or("PivotY",     0.0);
    let cog_h         = settings.child_f64_or("CofGH",      0.0);
    let cog_v         = settings.child_f64_or("CofGV",      0.0);
    let data_cog_x    = settings.child_f64_or("DataCofGX",  0.0);
    let hd_adj        = settings.child_text("HdAdj").map(str::to_owned);
    let fork_ht_ref   = settings.child_text("ForkHtRef").map(str::to_owned);
    let upr_hd_adj    = settings.child_f64_or("UprHdAdj",   0.0);
    let lwr_hd_adj    = settings.child_f64_or("LwrHdAdj",   0.0);

    let dual_shock    = settings.child_bool("boolDualRrShock");
    let shock_l_ext   = settings.child_f64_or("ShockLExt",  0.0);
    let spring_rate   = settings.child_f64_or("SpringRate",  0.0);
    let preload       = settings.child_f64_or("Preload",     0.0);
    let topout_l      = settings.child_f64_or("TopoutL",     0.0);
    let topout_rate   = settings.child_f64_or("TopoutRate",  0.0);

    // Fork spring/top-out settings come from Settings (not from ForkInstance)
    let fork_l_spr_rate   = settings.child_f64_or("ForkLSprRate", 0.0);
    let fork_r_spr_rate   = settings.child_f64_or("ForkRSprRate", 0.0);
    let fork_l_spr_pre_l  = settings.child_f64_or("ForkLSprPreL", 0.0);
    let fork_r_spr_pre_l  = settings.child_f64_or("ForkRSprPreL", 0.0);
    let fork_l_oil_level  = settings.child_f64_or("ForkLOilLevel", 0.0);
    let fork_r_oil_level  = settings.child_f64_or("ForkROilLevel", 0.0);
    let fork_l_top_rate   = settings.child_f64_or("ForkLTopRate", 0.0);
    let fork_r_top_rate   = settings.child_f64_or("ForkRTopRate", 0.0);
    let fork_l_top_l      = settings.child_f64_or("ForkLTopL",    0.0);
    let fork_r_top_l      = settings.child_f64_or("ForkRTopL",    0.0);

    // Optional design ride height
    let ride_ht_ref = settings.child_text("RideHtRef").map(str::to_owned);
    let design_axle_below_pivot = if is_vertical_pivot_axle_ref(ride_ht_ref.as_deref()) {
        settings.child_f64("RideHtPtV").filter(|&v| v < -10.0 && v > -400.0)
    } else {
        None
    };

    // Tire profiles (selected by index)
    let (fr_rad, fr_major, fr_minor) =
        tire_profile(components, "FrTire", "FrTireInstance", sel_fr_tire);
    let (rr_rad, rr_major, rr_minor) =
        tire_profile(components, "RrTire", "RrTireInstance", sel_rr_tire);
    let front_tire_rad = if fr_rad <= 0.0 { whl * 0.2 } else { fr_rad };
    let rear_tire_rad  = if rr_rad <= 0.0 { whl * 0.22 } else { rr_rad };

    // Frame — take the first FrameInstance
    let frame_inst = components
        .and_then(|c| c.child("Frame"))
        .and_then(|f| f.children.first())
        .map(|el| FrameInstance {
            head_angle_deg: el.child_f64_or("HeadAngle",  0.0),
            head_x:         el.child_f64_or("HeadX",      0.0),
            head_y:         el.child_f64_or("HeadY",      0.0),
            head_ht:        el.child_f64_or("HeadHt",     0.0),
            link_mnt_x:     el.child_f64_or("LinkMntX",   0.0),
            link_mnt_y:     el.child_f64_or("LinkMntY",   0.0),
            shock_mnt_x:    el.child_f64_or("ShockMntX",  0.0),
            shock_mnt_y:    el.child_f64_or("ShockMntY",  0.0),
            c_shaft_x:      el.child_f64_or("CShaftX",    0.0),
            c_shaft_y:      el.child_f64_or("CShaftY",    0.0),
        });

    // Swingarm — first SwingarmInstance
    let swingarm_inst = components
        .and_then(|c| c.child("Swingarm"))
        .and_then(|s| s.children.first())
        .map(|el| SwingarmInstance {
            offset:     el.child_f64_or("Offset",     0.0),
            link_x:     el.child_f64_or("LinkX",      0.0),
            link_y:     el.child_f64_or("LinkY",      0.0),
            shock_x:    el.child_f64_or("ShockX",     0.0),
            shock_y:    el.child_f64_or("ShockY",     0.0),
            ecc_radius: el.child_f64_or("EccRadius",  0.0),
        });

    // Fork — selected by index
    let fork_inst = components
        .and_then(|c| c.child("Fork"))
        .and_then(|f| f.child_by_index("ForkInstance", sel_fork))
        .map(|el| {
            let (oil_levels, air_volumes) = parse_oil_air_table(el);
            ForkInstance {
                length_mm:                el.child_f64_or("L",               0.0),
                upr_tube_l_mm:            el.child_f64_or("UprTubeL",        0.0),
                travel_mm:                el.child_f64_or("Travel",          0.0),
                lwr_offset_mm:            el.child_f64_or("LwrOffset",       0.0),
                fork_l_spr_rate_n_per_mm: fork_l_spr_rate,
                fork_r_spr_rate_n_per_mm: fork_r_spr_rate,
                fork_l_spr_pre_l_mm:      fork_l_spr_pre_l,
                fork_r_spr_pre_l_mm:      fork_r_spr_pre_l,
                fork_l_oil_level_mm:      fork_l_oil_level,
                fork_r_oil_level_mm:      fork_r_oil_level,
                fork_l_top_rate_n_per_mm: fork_l_top_rate,
                fork_r_top_rate_n_per_mm: fork_r_top_rate,
                fork_l_top_l_mm:          fork_l_top_l,
                fork_r_top_l_mm:          fork_r_top_l,
                bump_rate_n_per_mm:       el.child_f64_or("BumpRate", 0.0),
                bump_l_mm:                el.child_f64_or("BumpL",    0.0),
                air_spring_mode:          parse_air_spring_mode(el.child_text("AirSpring")),
                tube_dia_mm:              el.child_f64_or("TubeDia",       0.0),
                rod_dia_mm:               el.child_f64_or("RodDia",        0.0),
                p_bar:                    el.child_f64_or("P",             1.0),
                kappa:                    el.child_f64_or("Kappa",         1.4),
                oil_levels,
                air_volumes,
                cartridge_type:           parse_cartridge_type(el.child_text("Cartridge")),
                rod_thru_res_piston:      el.child_bool("RodThruResPiston"),
                asym_res:                 el.child_bool("AsymRes"),
                res_pressure_bar:         el.child_f64_or("ResP",             0.0),
                res_piston_dia_mm:        el.child_f64_or("ResPistonDia",     0.0),
                res_piston_dia_inner_mm:  el.child_f64_or("ResPistonDiaInner", 0.0),
                res_spring_rate_n_per_mm: el.child_f64_or("ResSpringRate",    0.0),
                res_spring_preload_mm:    el.child_f64_or("ResSpringPreload", 0.0),
                res_spring_chamber_vol_cc:el.child_f64_or("ResSpringChamberVol", 0.0),
            }
        });

    // Shock — first ShockInstance; length and spring rate come from Settings
    let shock_inst = components
        .and_then(|c| c.child("Shock"))
        .and_then(|s| s.children.first())
        .map(|el| ShockInstance {
            length_extended_mm:   shock_l_ext,
            spring_rate_n_per_mm: spring_rate,
            stroke_mm:            el.child_f64_or("Stroke",    75.0),
            shock_type:           el.child_text("Shock").map(str::to_owned),
            res_pressure_bar:     el.child_f64_or("ResP",       0.0),
            res_vol_cc:           el.child_f64_or("ResVol",     0.0),
            rod_dia_mm:           el.child_f64_or("RodDia",     0.0),
            bump_ht_mm:           el.child_f64_or("BumpHt",     0.0),
            bump_rate_n_per_mm:   el.child_f64_or("BumpRate",   0.0),
        });

    // Link — selected by index
    let link_inst = components
        .and_then(|c| c.child("Link"))
        .and_then(|l| l.child_by_index("LinkInstance", sel_link))
        .map(|el| LinkInstanceRecord {
            name:                el.child_text("Name").map(str::to_owned),
            anchor_shock:        el.child_f64_or("AnchorShock",   0.0),
            anchor_linkarm:      el.child_f64_or("AnchorLinkarm", 0.0),
            shock_linkarm:       el.child_f64_or("ShockLinkarm",  0.0),
            nom_linkarm_l:       el.child_f64_or("NomLinkarmL",   0.0),
            nom_shock_l:         el.child_f64_or("NomShockL",     0.0),
            rocker_orientation:  el.child_text("RockerOrientation").map(str::to_owned),
        });

    // Yoke — selected by index
    let yoke_inst = components
        .and_then(|c| c.child("Yoke"))
        .and_then(|y| y.child_by_index("YokeInstance", sel_yoke))
        .map(|el| YokeInstance {
            upr_yoke_ht: el.child_f64_or("UprYokeHt", 0.0),
            lwr_yoke_ht: el.child_f64_or("LwrYokeHt", 0.0),
        });

    Ok(ChassisModel {
        comments,
        motospec_version:            version,
        link_type,
        wheel_center_hypotenuse_mm:  whl,
        fr_sprocket_teeth:           fr_sprocket,
        rr_sprocket_teeth:           rr_sprocket,
        chain_pitch_raw,
        chain_pitch_mm,
        selected_fr_tire_index:      sel_fr_tire,
        selected_rr_tire_index:      sel_rr_tire,
        selected_link_index:         sel_link,
        selected_fork_index:         sel_fork,
        selected_yoke_index:         sel_yoke,
        fork_pos_mm:                 fork_pos,
        front_tire_rad_mm:           front_tire_rad,
        rear_tire_rad_mm:            rear_tire_rad,
        fr_tire_major_rad_mm:        fr_major,
        fr_tire_minor_rad_mm:        fr_minor,
        rr_tire_major_rad_mm:        rr_major,
        rr_tire_minor_rad_mm:        rr_minor,
        sw_l_mm:                     sw_l,
        ride_ht_ref,
        design_axle_below_pivot_mm:  design_axle_below_pivot,
        yoke_offset_mm:              yoke_offset,
        upr_hd_adj_mm:               upr_hd_adj,
        lwr_hd_adj_mm:               lwr_hd_adj,
        hd_adj,
        fork_ht_ref,
        pivot_x_mm:                  pivot_x,
        pivot_y_mm:                  pivot_y,
        cof_g_h:                     cog_h,
        cof_g_v:                     cog_v,
        data_cof_g_x:                data_cog_x,
        shock_preload_mm:            preload,
        topout_l_mm:                 topout_l,
        topout_rate_n_per_mm:        topout_rate,
        dual_rr_shock:               dual_shock,
        frame:    frame_inst,
        swingarm: swingarm_inst,
        fork:     fork_inst,
        shock:    shock_inst,
        link:     link_inst,
        yoke:     yoke_inst,
    })
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn tire_profile(
    components: Option<&XmlNode>,
    group: &str,
    instance_name: &str,
    idx: i32,
) -> (f64, f64, f64) {
    match components
        .and_then(|c| c.child(group))
        .and_then(|g| g.child_by_index(instance_name, idx))
    {
        None => (0.0, 0.0, 0.0),
        Some(el) => (
            el.child_f64_or("Rad",      0.0),
            el.child_f64_or("MajorRad", 0.0),
            el.child_f64_or("MinorRad", 0.0),
        ),
    }
}

fn parse_oil_air_table(fork_el: &XmlNode) -> (Vec<f64>, Vec<f64>) {
    let mut oils = Vec::new();
    let mut airs = Vec::new();
    for i in 1_usize..=50 {
        let oil = fork_el.child_f64(&format!("Oil{i}"));
        let air = fork_el.child_f64(&format!("Air{i}"));
        if let (Some(o), Some(a)) = (oil, air) {
            if o > 0.0 && a > 0.0 {
                oils.push(o);
                airs.push(a);
            }
        }
    }
    (oils, airs)
}

fn resolve_chain_pitch_mm(raw: Option<&str>) -> f64 {
    match raw {
        None | Some("") => 15.875,
        Some(s) => {
            if let Ok(mm) = s.parse::<f64>() {
                if mm > 1.0 && mm < 50.0 {
                    return mm;
                }
            }
            match s.to_uppercase().as_str() {
                "FOUR" | "428"                 => 12.7,
                "FIVE" | "520" | "525" | "530" => 15.875,
                "SIX"  | "630"                 => 19.05,
                "EIGHT" | "EIGHTH"             => 25.4,
                _                              => 15.875,
            }
        }
    }
}

fn is_vertical_pivot_axle_ref(ref_type: Option<&str>) -> bool {
    matches!(
        ref_type,
        Some("VERTICAL_PIVOT-AXLE") | Some("VERTICAL_WHEEL_POSITION")
    )
}

fn parse_air_spring_mode(raw: Option<&str>) -> AirSpringMode {
    match raw.map(str::to_uppercase).as_deref() {
        Some("NOMINAL_OIL_LEVEL") => AirSpringMode::NominalOilLevel,
        Some("OIL_LEVEL_TABLE")   => AirSpringMode::OilLevelTable,
        Some("FORK_VOLUME")       => AirSpringMode::ForkVolume,
        _                         => AirSpringMode::Unknown,
    }
}

fn parse_cartridge_type(raw: Option<&str>) -> CartridgeType {
    match raw.map(str::to_uppercase).as_deref() {
        Some("GAS")          => CartridgeType::Gas,
        Some("GAS_AND_SPRING") => CartridgeType::GasAndSpring,
        Some("SPRING")       => CartridgeType::Spring,
        Some("SPRING_MECH")  => CartridgeType::SpringMech,
        Some("THRU_ROD")     => CartridgeType::ThruRod,
        _                    => CartridgeType::Unknown,
    }
}
