//! Motorcycle chassis geometry: model, solved frame state, and drawing primitives.
//!
//! Parses MotoSPEC MS1/MS3 chassis definition files, builds precomputed suspension curves,
//! solves per-sample kinematics from pot readings, and emits side-view schematic primitives
//! for animated wireframe rendering.

pub mod motospec_parser;
pub mod side_view;
pub mod solver;
pub mod xml_loader;

// ---------------------------------------------------------------------------
// Model types — static chassis geometry loaded from an MS1/MS3 file
// ---------------------------------------------------------------------------

/// Air spring mode for the front fork.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AirSpringMode {
    #[default]
    Unknown,
    NominalOilLevel,
    OilLevelTable,
    ForkVolume,
}

/// Front fork cartridge type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CartridgeType {
    #[default]
    Unknown,
    Gas,
    GasAndSpring,
    Spring,
    SpringMech,
    ThruRod,
}

/// Static chassis definition loaded from a MotoSPEC chassis file.
#[derive(Debug, Clone, Default)]
pub struct ChassisModel {
    pub comments: Option<String>,
    pub motospec_version: Option<String>,
    pub link_type: Option<String>,
    /// Wheel-centre hypotenuse (mm): diagonal distance between front and rear axle centres.
    pub wheel_center_hypotenuse_mm: f64,
    pub fr_sprocket_teeth: i32,
    pub rr_sprocket_teeth: i32,
    /// Raw chain pitch string from the file (e.g. "FIVE" for 520).
    pub chain_pitch_raw: Option<String>,
    /// Resolved roller pitch in mm.
    pub chain_pitch_mm: f64,
    pub selected_fr_tire_index: i32,
    pub selected_rr_tire_index: i32,
    pub selected_link_index: i32,
    pub selected_fork_index: i32,
    pub selected_yoke_index: i32,
    /// Fork position adjuster (mm).
    pub fork_pos_mm: f64,
    pub front_tire_rad_mm: f64,
    pub rear_tire_rad_mm: f64,
    /// Front tire major section radius for lean calculations (mm).
    pub fr_tire_major_rad_mm: f64,
    pub fr_tire_minor_rad_mm: f64,
    pub rr_tire_major_rad_mm: f64,
    pub rr_tire_minor_rad_mm: f64,
    /// Nominal swingarm length (mm).
    pub sw_l_mm: f64,
    /// Ride height reference type string (e.g. "VERTICAL_PIVOT-AXLE").
    pub ride_ht_ref: Option<String>,
    /// Triple-clamp / fork offset (mm).
    pub yoke_offset_mm: f64,
    /// Design vertical drop from swingarm pivot to rear axle (mm, negative = axle below pivot).
    pub design_axle_below_pivot_mm: Option<f64>,
    /// Head-angle adjuster mode (e.g. "MID", "OFFSETS").
    pub hd_adj: Option<String>,
    /// Fork height reference mode (e.g. "UPPER").
    pub fork_ht_ref: Option<String>,
    /// Upper head adjuster position (mm).
    pub upr_hd_adj_mm: f64,
    /// Lower head adjuster position (mm).
    pub lwr_hd_adj_mm: f64,
    /// Centre-of-gravity horizontal offset (mm).
    pub cof_g_h: f64,
    /// Centre-of-gravity vertical offset (mm).
    pub cof_g_v: f64,
    /// Design CoG X distance (mm).
    pub data_cof_g_x: f64,
    /// Swingarm pivot X adjustment (mm).
    pub pivot_x_mm: f64,
    /// Swingarm pivot Y adjustment (mm).
    pub pivot_y_mm: f64,
    /// Rear shock preload (mm).
    pub shock_preload_mm: f64,
    /// Rear top-out spring engagement length (mm).
    pub topout_l_mm: f64,
    /// Rear top-out spring rate (N/mm).
    pub topout_rate_n_per_mm: f64,
    /// Dual rear shock configuration.
    pub dual_rr_shock: bool,
    pub frame: Option<FrameInstance>,
    pub swingarm: Option<SwingarmInstance>,
    pub fork: Option<ForkInstance>,
    pub shock: Option<ShockInstance>,
    pub link: Option<LinkInstanceRecord>,
    pub yoke: Option<YokeInstance>,
}

impl ChassisModel {
    /// Returns true when all four tire section radii are present (enables lean corrections).
    pub fn has_elliptical_tire_data(&self) -> bool {
        self.fr_tire_major_rad_mm > 0.0
            && self.fr_tire_minor_rad_mm > 0.0
            && self.rr_tire_major_rad_mm > 0.0
            && self.rr_tire_minor_rad_mm > 0.0
    }
}

/// Frame geometry instance.
#[derive(Debug, Clone, Default)]
pub struct FrameInstance {
    pub head_angle_deg: f64,
    pub head_x: f64,
    pub head_y: f64,
    pub head_ht: f64,
    pub link_mnt_x: f64,
    pub link_mnt_y: f64,
    pub shock_mnt_x: f64,
    pub shock_mnt_y: f64,
    pub c_shaft_x: f64,
    pub c_shaft_y: f64,
}

/// Swingarm geometry instance.
#[derive(Debug, Clone, Default)]
pub struct SwingarmInstance {
    /// Axle slot vertical drop from the swingarm body axis (mm).
    pub offset: f64,
    pub link_x: f64,
    pub link_y: f64,
    pub shock_x: f64,
    pub shock_y: f64,
    pub ecc_radius: f64,
}

/// Fork geometry and spring/damper instance.
#[derive(Debug, Clone, Default)]
pub struct ForkInstance {
    pub length_mm: f64,
    pub upr_tube_l_mm: f64,
    pub travel_mm: f64,
    pub lwr_offset_mm: f64,
    pub fork_l_spr_rate_n_per_mm: f64,
    pub fork_r_spr_rate_n_per_mm: f64,
    pub fork_l_spr_pre_l_mm: f64,
    pub fork_r_spr_pre_l_mm: f64,
    pub fork_l_top_rate_n_per_mm: f64,
    pub fork_r_top_rate_n_per_mm: f64,
    pub fork_l_top_l_mm: f64,
    pub fork_r_top_l_mm: f64,
    pub fork_l_oil_level_mm: f64,
    pub fork_r_oil_level_mm: f64,
    pub bump_rate_n_per_mm: f64,
    pub bump_l_mm: f64,
    pub air_spring_mode: AirSpringMode,
    pub tube_dia_mm: f64,
    pub rod_dia_mm: f64,
    pub p_bar: f64,
    pub kappa: f64,
    pub oil_levels: Vec<f64>,
    pub air_volumes: Vec<f64>,
    pub cartridge_type: CartridgeType,
    pub rod_thru_res_piston: bool,
    pub asym_res: bool,
    pub res_pressure_bar: f64,
    pub res_piston_dia_mm: f64,
    pub res_piston_dia_inner_mm: f64,
    pub res_spring_rate_n_per_mm: f64,
    pub res_spring_preload_mm: f64,
    pub res_spring_chamber_vol_cc: f64,
}

/// Rear shock geometry and damper instance.
#[derive(Debug, Clone, Default)]
pub struct ShockInstance {
    pub length_extended_mm: f64,
    pub spring_rate_n_per_mm: f64,
    pub stroke_mm: f64,
    pub shock_type: Option<String>,
    pub res_pressure_bar: f64,
    pub res_vol_cc: f64,
    pub rod_dia_mm: f64,
    pub bump_ht_mm: f64,
    pub bump_rate_n_per_mm: f64,
}

/// Rear linkage instance (rocker / SeeSaw geometry).
#[derive(Debug, Clone, Default)]
pub struct LinkInstanceRecord {
    pub name: Option<String>,
    pub anchor_shock: f64,
    pub anchor_linkarm: f64,
    pub shock_linkarm: f64,
    pub nom_linkarm_l: f64,
    pub rocker_orientation: Option<String>,
    /// Nominal shock length override (mm); when > 0 replaces Settings ShockLExt.
    pub nom_shock_l: f64,
}

/// Yoke (triple-clamp) geometry instance.
#[derive(Debug, Clone, Default)]
pub struct YokeInstance {
    pub upr_yoke_ht: f64,
    pub lwr_yoke_ht: f64,
}

// ---------------------------------------------------------------------------
// FrameState — per-sample kinematics produced by the solver
// ---------------------------------------------------------------------------

/// Instantaneous chassis geometry solved from suspension pot readings.
///
/// All linear dimensions in mm, angles in degrees unless noted.
#[derive(Debug, Clone, Default)]
pub struct FrameState {
    // Inputs
    pub rr_pot_mm: f64,
    pub fr_pot_mm: f64,

    // Rear suspension
    /// Swingarm angle relative to world horizontal (deg); negative when axle is below pivot.
    pub inst_sw_angle_deg: f64,
    pub rr_wheel_travel_mm: f64,
    pub rr_wheel_force_n: f64,
    pub rr_wheel_rate_n_per_mm: f64,
    pub rr_motion_ratio_shock_per_wheel: f64,
    pub rr_motion_ratio_wheel_per_shock: f64,

    // Rear ride height
    pub inst_ride_ht_mm: f64,

    // Front suspension
    pub fr_fork_comp_mm: f64,
    pub fr_wheel_comp_mm: f64,
    pub fr_fork_force_n: f64,
    pub fr_fork_rate_n_per_mm: f64,
    pub fr_wheel_force_n: f64,
    pub fr_wheel_rate_n_per_mm: f64,

    // Geometry
    pub wheelbase_mm: f64,
    pub rake_deg: f64,
    /// Ground trail (mm) = normal trail / cos(rake).
    pub ground_trail_mm: f64,
    /// Normal trail (mm).
    pub trail_mm: f64,
    pub front_axle_height_mm: f64,
    pub rear_axle_height_mm: f64,
    pub pivot_height_mm: f64,
    pub ground_angle_deg: f64,

    // Anti-squat / load transfer
    pub instant_center_height_mm: f64,
    pub anti_squat_pct: f64,
    pub anti_squat_angle_deg: f64,
    pub anti_squat_tangent: f64,
    pub load_transfer_angle_deg: f64,
    pub load_transfer_tangent: f64,

    // CoG
    pub cog_x_mm: f64,
    pub cog_y_mm: f64,
    pub cog_percent_front: f64,
    pub cog_percent_rear: f64,

    // Schematic rendering angles and positions
    /// Swingarm body rotation angle for wireframe rendering (rad).
    pub gamma_rad: f64,
    /// Frame pitch angle for wireframe rendering (rad).
    pub theta_rad: f64,
    pub rear_axle_x: f64,
    pub rear_axle_y: f64,
    pub pivot_x: f64,
    pub pivot_y: f64,
}

// ---------------------------------------------------------------------------
// Suspension curve types — precomputed during chassis preparation
// ---------------------------------------------------------------------------

/// One row of the precomputed rear suspension table.
#[derive(Debug, Clone, Copy, Default)]
pub struct RearSuspRow {
    /// Swingarm angle (deg), adjusted for SLIDER eccentricity.
    pub swingarm_angle_deg: f64,
    /// Shock pot abscissa (mm): `ShockLExt_at_topped − shock_length`.
    pub shock_pot_mm: f64,
    /// Rear wheel travel vs topped position (mm).
    pub wheel_travel_mm: f64,
    pub motion_ratio_shock_per_wheel: f64,
    pub motion_ratio_wheel_per_shock: f64,
    pub wheel_force_n: f64,
    pub wheel_rate_n_per_mm: f64,
}

/// Precomputed rear suspension curve (1201-row swingarm angle sweep).
#[derive(Debug, Clone, Default)]
pub struct RearSuspCurve {
    pub rows: Vec<RearSuspRow>,
    /// Effective swingarm length (mm).
    pub eff_sw_l_mm: f64,
    /// Shock length at the topped row (mm).
    pub shock_l_ext_topped_mm: f64,
}

/// One row of the precomputed fork force table.
#[derive(Debug, Clone, Copy, Default)]
pub struct ForkForceRow {
    /// Fork compression (mm), starting at −5 up through travel.
    pub fork_comp_mm: f64,
    pub fork_comp_wheel_mm: f64,
    pub spring_force_n: f64,
    pub air_force_n: f64,
    pub topout_bump_force_n: f64,
    pub reservoir_force_n: f64,
    pub total_fork_force_n: f64,
    pub fork_rate_n_per_mm: f64,
}

/// Precomputed fork force curve.
#[derive(Debug, Clone, Default)]
pub struct ForkForceCurve {
    pub rows: Vec<ForkForceRow>,
    /// Index into rows where fork_comp_mm ≈ 0 (fully extended).
    pub topped_index: usize,
    /// Rake angle used to project fork forces (rad).
    pub rake_rad: f64,
}
