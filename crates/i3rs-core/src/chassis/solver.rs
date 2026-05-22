//! Per-sample chassis kinematics solver.
//!
//! [`ChassisSolver::prepare`] builds the precomputed suspension curves from a [`ChassisModel`].
//! [`ChassisSolver::solve`] converts rear and front pot readings plus lean angle into a
//! fully populated [`FrameState`] containing all instantaneous geometry values.

use std::f64::consts::PI;

use super::{
    AirSpringMode, CartridgeType, ChassisModel, ForkForceCurve, ForkForceRow, FrameState,
    RearSuspCurve, RearSuspRow,
};

// ---------------------------------------------------------------------------
// Public solver entry point
// ---------------------------------------------------------------------------

/// Pre-prepared chassis geometry solver.
pub struct ChassisSolver {
    model: ChassisModel,
    rear_curve: Option<RearSuspCurve>,
    fork_curve: Option<ForkForceCurve>,
    sw_l: f64,
    axle_offset: f64,
    lx_h: f64,
    rear_rad: f64,
    front_rad: f64,
    fr_axle_full_ext: FrontAxleFullExtension,
    /// Design-pose fork alignment (reserved for side-view schematic rendering).
    #[allow(dead_code)]
    alignment_at_zero: ForkAlignment,
}

impl ChassisSolver {
    /// Build all precomputed curves and cache design-pose values.
    pub fn prepare(model: ChassisModel) -> Self {
        let rear_curve = build_rear_curve(&model);
        let fork_curve = build_fork_curve(&model);
        let align0 = fork_alignment_calcs(&model, 0.0);
        let fr_axle_full_ext = front_geo(&model, &align0);

        let sw_l = rear_curve.as_ref().map(|c| c.eff_sw_l_mm).unwrap_or(model.sw_l_mm);
        let axle_offset = model.swingarm.as_ref().map(|s| s.offset).unwrap_or(0.0);
        let lx_h = if sw_l > axle_offset {
            (sw_l * sw_l - axle_offset * axle_offset).sqrt()
        } else {
            sw_l
        };
        let rear_rad = model.rear_tire_rad_mm;
        let front_rad = model.front_tire_rad_mm;

        ChassisSolver {
            model,
            rear_curve,
            fork_curve,
            sw_l,
            axle_offset,
            lx_h,
            rear_rad,
            front_rad,
            fr_axle_full_ext,
            alignment_at_zero: align0,
        }
    }

    /// Solve instantaneous geometry from suspension pot readings and lean angle.
    pub fn solve(&self, rr_pot_mm: f64, fr_pot_mm: f64, lean_deg: f64) -> FrameState {
        let mut inst_sw_angle_deg = 0.0_f64;
        let mut rr_wheel_travel = 0.0_f64;
        let mut rr_wheel_force = 0.0_f64;
        let mut rr_wheel_rate = 0.0_f64;
        let mut rr_mr_sw = 1.0_f64;
        let mut rr_mr_ws = 1.0_f64;

        if let Some(ref curve) = self.rear_curve {
            lookup_closest(
                curve,
                rr_pot_mm,
                &mut inst_sw_angle_deg,
                &mut rr_wheel_travel,
                &mut rr_wheel_force,
                &mut rr_wheel_rate,
                &mut rr_mr_sw,
                &mut rr_mr_ws,
            );
        }

        let alignment_at_fr_pot = fork_alignment_calcs(&self.model, fr_pot_mm);
        let mut pose = compute_pose(
            &self.model,
            &self.fr_axle_full_ext,
            &alignment_at_fr_pot,
            &alignment_at_fr_pot,
            fr_pot_mm,
            inst_sw_angle_deg,
            self.sw_l,
            self.front_rad,
            self.rear_rad,
        );

        pose = apply_lean_corrections(&self.model, pose, &alignment_at_fr_pot, self.sw_l, lean_deg);

        let (fr_tire_rad, rr_tire_rad) = effective_tire_radii_mm(&self.model, lean_deg);

        let fr_fork_comp = fr_pot_mm;
        let mut fr_wheel_comp = 0.0_f64;
        let mut fr_fork_force = 0.0_f64;
        let mut fr_fork_rate = 0.0_f64;
        let mut fr_wheel_force = 0.0_f64;
        let mut fr_wheel_rate = 0.0_f64;

        if let Some(ref fc) = self.fork_curve {
            lookup_at_fr_pot(
                fc,
                fr_fork_comp,
                pose.inst_fork_angle_deg,
                &mut fr_wheel_force,
                &mut fr_wheel_rate,
                &mut fr_fork_force,
                &mut fr_fork_rate,
                &mut fr_wheel_comp,
            );
        }

        let chassis = compute_chassis_calcs(
            &self.model,
            pose.wheelbase_mm,
            pose.inst_sw_angle_deg,
            pose.ground_angle_deg,
            fr_tire_rad,
            rr_tire_rad,
            self.sw_l,
        );

        let theta_rad = pose.ground_angle_deg * PI / 180.0;
        let gamma_rad = self.compute_gamma(pose.inst_sw_angle_deg);

        FrameState {
            rr_pot_mm,
            fr_pot_mm,
            inst_sw_angle_deg: pose.inst_sw_angle_deg,
            rr_wheel_travel_mm: rr_wheel_travel,
            rr_wheel_force_n: rr_wheel_force,
            rr_wheel_rate_n_per_mm: rr_wheel_rate,
            rr_motion_ratio_shock_per_wheel: rr_mr_sw,
            rr_motion_ratio_wheel_per_shock: rr_mr_ws,
            inst_ride_ht_mm: pose.inst_ride_ht_mm,
            fr_fork_comp_mm: fr_fork_comp,
            fr_wheel_comp_mm: fr_wheel_comp,
            fr_fork_force_n: fr_fork_force,
            fr_fork_rate_n_per_mm: fr_fork_rate,
            fr_wheel_force_n: fr_wheel_force,
            fr_wheel_rate_n_per_mm: fr_wheel_rate,
            wheelbase_mm: pose.wheelbase_mm,
            rake_deg: pose.inst_rake_deg,
            ground_trail_mm: pose.inst_ground_trail_mm,
            trail_mm: pose.inst_real_trail_mm,
            front_axle_height_mm: fr_tire_rad,
            rear_axle_height_mm: rr_tire_rad,
            pivot_height_mm: chassis.pivot_y,
            ground_angle_deg: pose.ground_angle_deg,
            instant_center_height_mm: chassis.ic_y,
            anti_squat_pct: chassis.anti_squat_percent,
            anti_squat_angle_deg: chassis.anti_squat_angle_deg,
            anti_squat_tangent: chassis.anti_squat_tangent,
            load_transfer_angle_deg: chassis.load_transfer_angle_deg,
            load_transfer_tangent: chassis.load_transfer_tangent,
            cog_x_mm: chassis.cog_x,
            cog_y_mm: chassis.cog_y,
            cog_percent_front: chassis.cog_percent_front,
            cog_percent_rear: chassis.cog_percent_rear,
            gamma_rad,
            theta_rad,
            rear_axle_x: chassis.rr_axle_x,
            rear_axle_y: chassis.rr_axle_y,
            pivot_x: chassis.pivot_x,
            pivot_y: chassis.pivot_y,
        }
    }

    fn compute_gamma(&self, inst_sw_angle_deg: f64) -> f64 {
        let phi = self.axle_offset.atan2(self.lx_h);
        inst_sw_angle_deg * PI / 180.0 + phi
    }
}

// ---------------------------------------------------------------------------
// Fork alignment calculations (ForkAlignmentCalcs)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct ForkAlignment {
    #[allow(dead_code)]
    head_angle_mod_deg: f64,
    adjusted_head_angle_deg: f64,
    adjusted_fork_angle_deg: f64,
    axle_dist_from_stem_mm: f64,
    axle_angle_from_stem_deg: f64,
}

#[derive(Clone, Debug, Default)]
struct FrontAxleFullExtension {
    x_mm: f64,
    y_mm: f64,
}

#[derive(Clone, Debug, Default)]
struct PoseScalars {
    /// Front axle position (reserved for side-view schematic rendering).
    #[allow(dead_code)]
    fr_axle_x: f64,
    #[allow(dead_code)]
    fr_axle_y: f64,
    inst_sw_angle_deg: f64,
    inst_rake_deg: f64,
    inst_fork_angle_deg: f64,
    ground_angle_deg: f64,
    wheel_inclination_deg: f64,
    tire_inclination_deg: f64,
    whl_ctr_hypotenuse_mm: f64,
    wheelbase_mm: f64,
    inst_real_trail_mm: f64,
    inst_ground_trail_mm: f64,
    inst_ride_ht_mm: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HdAdjMode {
    Offsets,
    Upper,
    Mid,
    Lower,
    ForkAngle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForkHtRefMode {
    Upper,
    Lower,
    UprTubeLwrYoke,
    AxleHeadstock,
    #[allow(dead_code)]
    Unknown,
}

fn parse_hd_adj(value: Option<&str>) -> HdAdjMode {
    match value.map(|v| v.to_ascii_uppercase()).as_deref() {
        Some("OFFSETS") => HdAdjMode::Offsets,
        Some("UPPER") => HdAdjMode::Upper,
        Some("MID") => HdAdjMode::Mid,
        Some("LOWER") => HdAdjMode::Lower,
        Some("FORKANGLE") => HdAdjMode::ForkAngle,
        _ => HdAdjMode::Mid,
    }
}

fn parse_fork_ht_ref(value: Option<&str>) -> ForkHtRefMode {
    match value.map(|v| v.to_ascii_uppercase()).as_deref() {
        Some("UPPER") => ForkHtRefMode::Upper,
        Some("LOWER") => ForkHtRefMode::Lower,
        Some("UPRTUBE_LWRYOKE") => ForkHtRefMode::UprTubeLwrYoke,
        Some("AXLE_HDSTK") => ForkHtRefMode::AxleHeadstock,
        _ => ForkHtRefMode::Upper,
    }
}

fn is_axle_headstock_ref(value: Option<&str>) -> bool {
    parse_fork_ht_ref(value) == ForkHtRefMode::AxleHeadstock
}

fn compute_fork_length_mm(model: &ChassisModel) -> f64 {
    let frame = match &model.frame { Some(f) => f, None => return 0.0 };
    let fork = match &model.fork { Some(f) => f, None => return 0.0 };
    let yoke = model.yoke.as_ref();
    let upr_yoke = yoke.map(|y| y.upr_yoke_ht).unwrap_or(0.0);
    let lwr_yoke = yoke.map(|y| y.lwr_yoke_ht).unwrap_or(0.0);
    let fork_pos = model.fork_pos_mm;
    let hd_adj = parse_hd_adj(model.hd_adj.as_deref());
    let fhref = parse_fork_ht_ref(model.fork_ht_ref.as_deref());

    match hd_adj {
        HdAdjMode::Offsets | HdAdjMode::Lower => match fhref {
            ForkHtRefMode::Upper => fork.length_mm - frame.head_ht - upr_yoke - fork_pos,
            ForkHtRefMode::Lower => lwr_yoke + fork_pos,
            ForkHtRefMode::UprTubeLwrYoke => lwr_yoke + fork_pos + fork.length_mm - fork.upr_tube_l_mm,
            _ => fork_pos,
        },
        HdAdjMode::Upper => match fhref {
            ForkHtRefMode::Upper => fork.length_mm - upr_yoke - fork_pos,
            ForkHtRefMode::Lower => frame.head_ht + lwr_yoke + fork_pos,
            ForkHtRefMode::UprTubeLwrYoke => {
                fork.length_mm - fork.upr_tube_l_mm + fork_pos + lwr_yoke + frame.head_ht
            }
            _ => fork_pos,
        },
        HdAdjMode::Mid => match fhref {
            ForkHtRefMode::Upper => fork.length_mm - frame.head_ht * 0.5 - upr_yoke - fork_pos,
            ForkHtRefMode::Lower => frame.head_ht * 0.5 + lwr_yoke + fork_pos,
            ForkHtRefMode::UprTubeLwrYoke => {
                fork.length_mm - fork.upr_tube_l_mm + fork_pos + lwr_yoke + frame.head_ht * 0.5
            }
            _ => fork_pos,
        },
        HdAdjMode::ForkAngle => match fhref {
            ForkHtRefMode::Upper => fork.length_mm - frame.head_ht - upr_yoke - fork_pos,
            ForkHtRefMode::Lower => lwr_yoke + fork_pos,
            _ => fork_pos,
        },
    }
}

fn fork_alignment_calcs(model: &ChassisModel, fr_pot_mm: f64) -> ForkAlignment {
    let frame = match &model.frame { Some(f) => f, None => return ForkAlignment::default() };
    let fork = match &model.fork { Some(f) => f, None => return ForkAlignment::default() };
    let fork_length = compute_fork_length_mm(model);
    let fr_pot = fr_pot_mm.max(0.0);
    let hd_adj = parse_hd_adj(model.hd_adj.as_deref());

    let (head_angle_mod, adjusted_head, adjusted_fork) = match hd_adj {
        HdAdjMode::Offsets => {
            let ang = ((model.lwr_hd_adj_mm - model.upr_hd_adj_mm) / frame.head_ht).atan();
            let m = deg(ang);
            (m, frame.head_angle_deg + m, frame.head_angle_deg + m)
        }
        HdAdjMode::ForkAngle => {
            let m = model.upr_hd_adj_mm;
            (m, frame.head_angle_deg, frame.head_angle_deg + m)
        }
        _ => {
            let m = model.upr_hd_adj_mm;
            (m, frame.head_angle_deg + m, frame.head_angle_deg + m)
        }
    };

    let yoke_plus_lwr = model.yoke_offset_mm + fork.lwr_offset_mm;

    if !is_axle_headstock_ref(model.fork_ht_ref.as_deref()) {
        let stem_len = fork_length - fr_pot;
        let axle_dist = (stem_len * stem_len + yoke_plus_lwr * yoke_plus_lwr).sqrt();
        let axle_ang = head_angle_mod + deg(yoke_plus_lwr.atan2(stem_len));
        return ForkAlignment {
            head_angle_mod_deg: head_angle_mod,
            adjusted_head_angle_deg: adjusted_head,
            adjusted_fork_angle_deg: adjusted_fork,
            axle_dist_from_stem_mm: axle_dist,
            axle_angle_from_stem_deg: axle_ang,
        };
    }

    let offset_along = yoke_plus_lwr / rad(head_angle_mod).cos();
    if offset_along < 0.0 {
        let stem_len = fork_length - fr_pot;
        let num3 = stem_len * stem_len + offset_along * offset_along;
        let num4 = 2.0 * stem_len * offset_along.abs();
        let ang = 90.0 - head_angle_mod;
        let axle_dist = (num3 - num4 * rad(ang).cos()).sqrt();
        let cos_arg = ((offset_along * offset_along - stem_len * stem_len - axle_dist * axle_dist)
            / (-2.0 * axle_dist * stem_len))
            .clamp(-1.0, 1.0);
        let axle_ang = head_angle_mod - deg(cos_arg.acos());
        ForkAlignment {
            head_angle_mod_deg: head_angle_mod,
            adjusted_head_angle_deg: adjusted_head,
            adjusted_fork_angle_deg: adjusted_fork,
            axle_dist_from_stem_mm: axle_dist,
            axle_angle_from_stem_deg: axle_ang,
        }
    } else {
        let stem_len = fork_length - fr_pot;
        let num5 = stem_len * stem_len + offset_along * offset_along;
        let num6 = 2.0 * stem_len * offset_along.abs();
        let ang = 90.0 + head_angle_mod;
        let axle_dist = (num5 - num6 * rad(ang).cos()).sqrt();
        let cos_arg = ((offset_along * offset_along - stem_len * stem_len - axle_dist * axle_dist)
            / (-2.0 * axle_dist * stem_len))
            .clamp(-1.0, 1.0);
        let axle_ang = head_angle_mod + deg(cos_arg.acos());
        ForkAlignment {
            head_angle_mod_deg: head_angle_mod,
            adjusted_head_angle_deg: adjusted_head,
            adjusted_fork_angle_deg: adjusted_fork,
            axle_dist_from_stem_mm: axle_dist,
            axle_angle_from_stem_deg: axle_ang,
        }
    }
}

fn front_geo(model: &ChassisModel, alignment_at_zero: &ForkAlignment) -> FrontAxleFullExtension {
    let frame = match &model.frame { Some(f) => f, None => return FrontAxleFullExtension::default() };
    let hd_adj = parse_hd_adj(model.hd_adj.as_deref()) as i32;
    let head_rad = rad(frame.head_angle_deg);
    let phi = rad(frame.head_angle_deg + alignment_at_zero.axle_angle_from_stem_deg);
    let sin_head = head_rad.sin();
    let cos_head = head_rad.cos();
    let sin_phi = phi.sin();
    let cos_phi = phi.cos();

    let mut base_x = cos_head * model.lwr_hd_adj_mm + frame.head_x.abs() + model.pivot_x_mm;
    let mut base_y = sin_head * model.lwr_hd_adj_mm + (frame.head_y - model.pivot_y_mm);

    if hd_adj == 1 {
        base_x -= sin_head * frame.head_ht;
        base_y += cos_head * frame.head_ht;
    } else if hd_adj == 2 {
        base_x -= sin_head * frame.head_ht * 0.5;
        base_y += cos_head * frame.head_ht * 0.5;
    }

    FrontAxleFullExtension {
        x_mm: base_x + sin_phi * alignment_at_zero.axle_dist_from_stem_mm,
        y_mm: base_y - cos_phi * alignment_at_zero.axle_dist_from_stem_mm,
    }
}

fn inst_real_trail_calc(
    model: &ChassisModel,
    alignment: &ForkAlignment,
    inst_rake_deg: f64,
    fr_tire_rad_mm: f64,
) -> (f64, f64) {
    let real = if parse_hd_adj(model.hd_adj.as_deref()) == HdAdjMode::ForkAngle {
        rad(inst_rake_deg).sin() * fr_tire_rad_mm
            - alignment.axle_dist_from_stem_mm * rad(alignment.axle_angle_from_stem_deg).sin()
    } else {
        let lwr = model.fork.as_ref().map(|f| f.lwr_offset_mm).unwrap_or(0.0);
        rad(inst_rake_deg).sin() * fr_tire_rad_mm - (model.yoke_offset_mm + lwr)
    };

    let cos_rake = rad(inst_rake_deg).cos();
    let ground = if cos_rake.abs() > 1e-9 { real / cos_rake } else { real };
    (real, ground)
}

fn compute_pose(
    model: &ChassisModel,
    full_ext: &FrontAxleFullExtension,
    alignment_for_slide: &ForkAlignment,
    alignment_for_trail: &ForkAlignment,
    fr_pot_mm: f64,
    inst_nom_sw_angle_deg: f64,
    eff_sw_l_mm: f64,
    fr_tire_rad_mm: f64,
    rr_tire_rad_mm: f64,
) -> PoseScalars {
    let fork_ang_rad = rad(alignment_for_slide.adjusted_fork_angle_deg);
    let fr_axle_x = full_ext.x_mm - fr_pot_mm * fork_ang_rad.sin();
    let fr_axle_y = full_ext.y_mm + fr_pot_mm * fork_ang_rad.cos();

    let sw_rad = rad(inst_nom_sw_angle_deg);
    let num4 = sw_rad.cos() * eff_sw_l_mm;
    let num5 = sw_rad.sin() * eff_sw_l_mm;
    let whl_ctr_hyp = ((num4 + fr_axle_x).powi(2) + (fr_axle_y - num5).powi(2)).sqrt();

    let wheel_incl_rad = ((fr_axle_y - num5) / whl_ctr_hyp).clamp(-1.0, 1.0).asin();
    let tire_incl_rad = ((fr_tire_rad_mm - rr_tire_rad_mm) / whl_ctr_hyp).clamp(-1.0, 1.0).asin();
    let wheel_incl_deg = deg(wheel_incl_rad);
    let tire_incl_deg = deg(tire_incl_rad);
    let ground_angle_deg = wheel_incl_deg - tire_incl_deg;

    let inst_sw_angle_deg = inst_nom_sw_angle_deg + ground_angle_deg;
    let inst_rake_deg = alignment_for_slide.adjusted_head_angle_deg - ground_angle_deg;
    let inst_fork_angle_deg = alignment_for_slide.adjusted_fork_angle_deg - ground_angle_deg;

    let (real_trail, ground_trail) =
        inst_real_trail_calc(model, alignment_for_trail, inst_rake_deg, fr_tire_rad_mm);
    let wheelbase_mm = whl_ctr_hyp * tire_incl_rad.cos();
    let inst_ride_ht = compute_ride_height(model, inst_sw_angle_deg, eff_sw_l_mm);

    PoseScalars {
        fr_axle_x,
        fr_axle_y,
        inst_sw_angle_deg,
        inst_rake_deg,
        inst_fork_angle_deg,
        ground_angle_deg,
        wheel_inclination_deg: wheel_incl_deg,
        tire_inclination_deg: tire_incl_deg,
        whl_ctr_hypotenuse_mm: whl_ctr_hyp,
        wheelbase_mm,
        inst_real_trail_mm: real_trail,
        inst_ground_trail_mm: ground_trail,
        inst_ride_ht_mm: inst_ride_ht,
    }
}

fn effective_tire_radii_mm(model: &ChassisModel, lean_deg: f64) -> (f64, f64) {
    let fr = model.front_tire_rad_mm;
    let rr = model.rear_tire_rad_mm;
    if lean_deg.abs() < 1e-9 || !model.has_elliptical_tire_data() {
        return (fr, rr);
    }
    let fr_scale = elliptical_major_scale(model.fr_tire_major_rad_mm, model.fr_tire_minor_rad_mm, lean_deg);
    let rr_scale = elliptical_major_scale(model.rr_tire_major_rad_mm, model.rr_tire_minor_rad_mm, lean_deg);
    let fr_out = (fr - model.fr_tire_major_rad_mm) + model.fr_tire_major_rad_mm * fr_scale;
    let rr_out = (rr - model.rr_tire_major_rad_mm) + model.rr_tire_major_rad_mm * rr_scale;
    (fr_out, rr_out)
}

fn apply_lean_corrections(
    model: &ChassisModel,
    mut pose: PoseScalars,
    alignment_for_trail: &ForkAlignment,
    eff_sw_l_mm: f64,
    lean_deg: f64,
) -> PoseScalars {
    if lean_deg.abs() < 1e-9 || !model.has_elliptical_tire_data() {
        return pose;
    }

    let (fr_tire_rad, _rr_tire_rad) = effective_tire_radii_mm(model, lean_deg);
    let fr_major = model.fr_tire_major_rad_mm;
    let fr_minor = model.fr_tire_minor_rad_mm;
    let rr_major = model.rr_tire_major_rad_mm;
    let rr_minor = model.rr_tire_minor_rad_mm;

    let fr_major_scale = elliptical_major_scale(fr_major, fr_minor, lean_deg);
    let rr_major_scale = elliptical_major_scale(rr_major, rr_minor, lean_deg);
    let fr_minor_scale = elliptical_minor_scale(fr_major, fr_minor, lean_deg);
    let rr_minor_scale = elliptical_minor_scale(rr_major, rr_minor, lean_deg);

    let fr_contact = fr_minor * fr_minor_scale;
    let rr_contact = rr_minor * rr_minor_scale;
    let lean_rad = rad(lean_deg);
    let tan_lean = lean_rad.tan();

    let num15 = fr_contact * tan_lean;
    let value21 = -(1.0 - fr_major_scale) * fr_major;
    let num17 = rr_contact * tan_lean;
    let value23 = -(1.0 - rr_major_scale) * rr_major;

    let lean_corr_deg = -deg(
        ((num17 + value23 - (num15 + value21)) / pose.whl_ctr_hypotenuse_mm).clamp(-1.0, 1.0).asin()
    );

    let ground_angle_deg = pose.wheel_inclination_deg - pose.tire_inclination_deg - lean_corr_deg;
    let inst_sw_angle_deg = pose.inst_sw_angle_deg - lean_corr_deg;
    let inst_rake_deg = pose.inst_rake_deg + lean_corr_deg;
    let inst_fork_angle_deg = pose.inst_fork_angle_deg + lean_corr_deg;

    let (real_trail, ground_trail) =
        inst_real_trail_calc(model, alignment_for_trail, inst_rake_deg, fr_tire_rad);
    let wheelbase_mm = pose.whl_ctr_hypotenuse_mm
        * rad(pose.tire_inclination_deg + lean_corr_deg).cos();
    let inst_ride_ht = compute_ride_height(model, inst_sw_angle_deg, eff_sw_l_mm);

    pose.inst_sw_angle_deg = inst_sw_angle_deg;
    pose.inst_rake_deg = inst_rake_deg;
    pose.inst_fork_angle_deg = inst_fork_angle_deg;
    pose.ground_angle_deg = ground_angle_deg;
    pose.wheelbase_mm = wheelbase_mm;
    pose.inst_real_trail_mm = real_trail;
    pose.inst_ground_trail_mm = ground_trail;
    pose.inst_ride_ht_mm = inst_ride_ht;
    pose
}

fn elliptical_major_scale(major_rad: f64, minor_rad: f64, lean_deg: f64) -> f64 {
    let lean_rad = rad(lean_deg);
    let cos_l = lean_rad.cos();
    let sin_l = lean_rad.sin();
    let projected = major_rad * cos_l;
    let denom = (major_rad * major_rad * cos_l * cos_l + minor_rad * minor_rad * sin_l * sin_l).sqrt();
    if denom > 1e-12 { projected / denom } else { 1.0 }
}

fn elliptical_minor_scale(major_rad: f64, minor_rad: f64, lean_deg: f64) -> f64 {
    let lean_rad = rad(lean_deg);
    let cos_l = lean_rad.cos();
    let sin_l = lean_rad.sin();
    let projected = minor_rad * sin_l;
    let denom = (major_rad * major_rad * cos_l * cos_l + minor_rad * minor_rad * sin_l * sin_l).sqrt();
    if denom > 1e-12 { projected / denom } else { 0.0 }
}

fn compute_ride_height(_model: &ChassisModel, inst_sw_angle_deg: f64, eff_sw_l_mm: f64) -> f64 {
    // VERTICAL_PIVOT-AXLE and VERTICAL_WHEEL_POSITION both reduce to the same formula.
    rad(inst_sw_angle_deg).sin() * eff_sw_l_mm
}

// ---------------------------------------------------------------------------
// Chassis calcs (anti-squat, CoG, load transfer)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ChassisResult {
    rr_axle_x: f64,
    rr_axle_y: f64,
    pivot_x: f64,
    pivot_y: f64,
    cog_x: f64,
    cog_y: f64,
    ic_y: f64,
    anti_squat_tangent: f64,
    anti_squat_angle_deg: f64,
    anti_squat_percent: f64,
    load_transfer_tangent: f64,
    load_transfer_angle_deg: f64,
    cog_percent_front: f64,
    cog_percent_rear: f64,
}

fn compute_chassis_calcs(
    model: &ChassisModel,
    wheelbase_mm: f64,
    inst_sw_angle_deg: f64,
    ground_angle_deg: f64,
    fr_tire_rad_mm: f64,
    rr_tire_rad_mm: f64,
    eff_sw_l_mm: f64,
) -> ChassisResult {
    let (cof_g_offset_x, cof_g_offset_y) =
        compute_cog_offset(model.cof_g_h, model.cof_g_v, ground_angle_deg);

    let rr_axle_y = rr_tire_rad_mm;
    let rr_axle_x = wheelbase_mm; // FrAxle alignment

    let sw_rad = rad(inst_sw_angle_deg);
    let pivot_move_x = -sw_rad.cos() * eff_sw_l_mm + rr_axle_x;
    let pivot_move_y = -sw_rad.sin() * eff_sw_l_mm + rr_axle_y;

    let (pivot_x, pivot_y) =
        compute_chassis_pivot(model.pivot_x_mm, model.pivot_y_mm, ground_angle_deg, pivot_move_x, pivot_move_y);

    let fr_axle_x = 0.0_f64;
    let _fr_axle_y = fr_tire_rad_mm;

    let frame = model.frame.as_ref();
    let (c_shaft_x, c_shaft_y) = if let Some(fr) = frame {
        let c_ang_deg = deg(fr.c_shaft_y.atan2(fr.c_shaft_x)) - ground_angle_deg;
        let c_len = (fr.c_shaft_x * fr.c_shaft_x + fr.c_shaft_y * fr.c_shaft_y).sqrt();
        let c_rad = rad(c_ang_deg);
        (pivot_x - c_rad.cos() * c_len, pivot_y + c_rad.sin() * c_len)
    } else {
        (pivot_x, pivot_y)
    };

    let chassis_cog_x = pivot_x + cof_g_offset_x;
    let chassis_cog_y = pivot_y + cof_g_offset_y;
    let data_cog_x = chassis_cog_x - fr_axle_x;

    let cog_percent_front = if wheelbase_mm > 0.0 {
        100.0 * (1.0 - data_cog_x / wheelbase_mm)
    } else {
        f64::NAN
    };
    let cog_percent_rear = if cog_percent_front.is_nan() {
        f64::NAN
    } else {
        100.0 - cog_percent_front
    };

    let (ic_x, ic_y, as_tangent, as_angle_deg, as_percent, lt_tangent, lt_angle_deg) =
        compute_anti_squat(
            model,
            wheelbase_mm,
            inst_sw_angle_deg,
            rr_axle_x,
            rr_axle_y,
            fr_axle_x,
            c_shaft_x,
            c_shaft_y,
            chassis_cog_y,
        );
    let _ = ic_x;

    ChassisResult {
        rr_axle_x,
        rr_axle_y,
        pivot_x,
        pivot_y,
        cog_x: chassis_cog_x,
        cog_y: chassis_cog_y,
        ic_y,
        anti_squat_tangent: as_tangent,
        anti_squat_angle_deg: as_angle_deg,
        anti_squat_percent: as_percent,
        load_transfer_tangent: lt_tangent,
        load_transfer_angle_deg: lt_angle_deg,
        cog_percent_front,
        cog_percent_rear,
    }
}

fn compute_cog_offset(cof_g_h: f64, cof_g_v: f64, ground_angle_deg: f64) -> (f64, f64) {
    if cof_g_h == 0.0 {
        return (0.0, 0.0);
    }
    let base_angle_deg = if cof_g_v > 0.0 {
        let num = (cof_g_v / cof_g_h).abs().atan();
        if cof_g_h > 0.0 { deg(num) + ground_angle_deg } else { deg(num) - ground_angle_deg }
    } else if cof_g_v < 0.0 {
        let num = (cof_g_v / cof_g_h).abs().atan();
        if cof_g_h > 0.0 { deg(num) - ground_angle_deg } else { deg(num) + ground_angle_deg }
    } else {
        0.0
    };
    let len = (cof_g_h * cof_g_h + cof_g_v * cof_g_v).sqrt();
    let r = rad(base_angle_deg);
    (
        if cof_g_h >= 0.0 { 1.0 } else { -1.0 } * r.cos() * len,
        if cof_g_v >= 0.0 { 1.0 } else { -1.0 } * r.sin() * len,
    )
}

fn compute_chassis_pivot(
    pivot_x_mm: f64,
    pivot_y_mm: f64,
    ground_angle_deg: f64,
    pivot_move_x: f64,
    pivot_move_y: f64,
) -> (f64, f64) {
    let pivot_ang_deg = deg(pivot_y_mm.atan2(pivot_x_mm)) + ground_angle_deg;
    let pivot_len = (pivot_x_mm * pivot_x_mm + pivot_y_mm * pivot_y_mm).sqrt();
    let r = rad(pivot_ang_deg);
    (pivot_move_x - r.cos() * pivot_len, pivot_move_y - r.sin() * pivot_len)
}

#[allow(clippy::too_many_arguments)]
fn compute_anti_squat(
    model: &ChassisModel,
    wheelbase_mm: f64,
    inst_sw_angle_deg: f64,
    rr_axle_x: f64,
    rr_axle_y: f64,
    fr_axle_x: f64,
    c_shaft_x: f64,
    c_shaft_y: f64,
    chassis_cog_y: f64,
) -> (f64, f64, f64, f64, f64, f64, f64) {
    let fr_p_dia = sprocket_pitch_diameter_mm(model.chain_pitch_mm, model.fr_sprocket_teeth, model.chain_pitch_raw.as_deref());
    let rr_p_dia = sprocket_pitch_diameter_mm(model.chain_pitch_mm, model.rr_sprocket_teeth, model.chain_pitch_raw.as_deref());

    let num34 = (rr_p_dia - fr_p_dia) * 0.5;
    let num35 = rr_axle_x - c_shaft_x;
    let num36 = rr_axle_y - c_shaft_y;
    let num37 = (num35 * num35 + num36 * num36).sqrt();
    if num37 < 1e-9 {
        return (rr_axle_x, rr_axle_y, 0.0, 0.0, f64::NAN, f64::NAN, f64::NAN);
    }

    let num34_clamped = (num34 / num37).clamp(-1.0, 1.0);
    let num38 = deg(num34_clamped.asin());
    // MotoSPEC uses atan(dy/dx), not atan2, for chain-line angle.
    let num39 = -deg((num36 / num35).atan()) - num38;
    let num40 = -inst_sw_angle_deg - num39;
    let sin_num40 = rad(num40).sin();
    if sin_num40.abs() < 1e-12 {
        return (rr_axle_x, rr_axle_y, 0.0, 0.0, f64::NAN, f64::NAN, f64::NAN);
    }

    let num41 = (rr_p_dia * 0.5) / sin_num40;
    let neg_sw_rad = rad(-inst_sw_angle_deg);
    let ic_x = rr_axle_x - num41 * neg_sw_rad.cos();
    let ic_y = rr_axle_y + num41 * neg_sw_rad.sin();

    let denom = rr_axle_x - ic_x;
    let as_tangent = if denom.abs() > 1e-12 { ic_y / denom } else { 0.0 };
    let as_angle_deg = deg(as_tangent.atan());

    if model.cof_g_h == 0.0
        || model.cof_g_v == 0.0
        || wheelbase_mm <= 0.0
        || chassis_cog_y.abs() < 1e-9
    {
        return (ic_x, ic_y, as_tangent, as_angle_deg, f64::NAN, f64::NAN, f64::NAN);
    }

    let lt_tangent = chassis_cog_y / wheelbase_mm;
    let lt_angle_deg = deg(lt_tangent.atan());
    let as_percent = rad(as_angle_deg).tan() * (rr_axle_x - fr_axle_x) / chassis_cog_y * 100.0;

    (ic_x, ic_y, as_tangent, as_angle_deg, as_percent, lt_tangent, lt_angle_deg)
}

fn sprocket_pitch_diameter_mm(chain_pitch_mm: f64, teeth: i32, chain_pitch_raw: Option<&str>) -> f64 {
    if teeth < 1 || chain_pitch_mm <= 0.0 {
        return 0.0;
    }
    let eleven = chain_pitch_raw
        .map(|r| r.trim().eq_ignore_ascii_case("ELEVEN"))
        .unwrap_or(false);
    if eleven {
        return teeth as f64 * chain_pitch_mm / PI;
    }
    let half_tooth_rad = rad(180.0 / teeth as f64);
    chain_pitch_mm / half_tooth_rad.sin()
}

// ---------------------------------------------------------------------------
// Rear suspension curve builder (Reversed_SeeSaw linkage)
// ---------------------------------------------------------------------------

fn build_rear_curve(model: &ChassisModel) -> Option<RearSuspCurve> {
    let frame = model.frame.as_ref()?;
    let sw = model.swingarm.as_ref()?;
    let sh = model.shock.as_ref()?;
    let lk = model.link.as_ref();

    if !model.link_type.as_deref()
        .map(|t| t.eq_ignore_ascii_case("Reversed_SeeSaw"))
        .unwrap_or(false)
    {
        return None;
    }

    const FULL_ROWS: usize = 1201;

    let swg_arm_link_angle = deg(sw.link_y.atan2(sw.link_x));
    let swg_arm_link_radius = hypot(sw.link_x, sw.link_y);
    let swg_arm_shock_angle = deg(sw.shock_y.atan2(sw.shock_x));
    let swg_arm_shock_radius = hypot(sw.shock_x, sw.shock_y);
    let swg_arm_link_angle_mod = swg_arm_link_angle;
    let swg_arm_shock_angle_mod = -swg_arm_shock_angle;

    let fr_x = frame.link_mnt_x - model.pivot_x_mm;
    let fr_y = frame.link_mnt_y - model.pivot_y_mm;

    let anchor_linkarm = lk.map(|l| l.anchor_linkarm).unwrap_or(0.0);
    let anchor_shock = lk.map(|l| l.anchor_shock).unwrap_or(0.0);
    let shock_linkarm = lk.map(|l| l.shock_linkarm).unwrap_or(0.0);
    let linkarm_l = lk.and_then(|l| if l.nom_linkarm_l > 0.0 { Some(l.nom_linkarm_l) } else { None })
        .unwrap_or(model.sw_l_mm * 0.1);
    let rocker_orientation = lk.and_then(|l| l.rocker_orientation.as_deref());

    let (col1, col2) = reversed_see_saw_sweep(
        fr_x, fr_y,
        swg_arm_link_angle_mod, swg_arm_link_radius,
        swg_arm_shock_angle_mod, swg_arm_shock_radius,
        anchor_linkarm, anchor_shock, shock_linkarm, linkarm_l,
        rocker_orientation,
    );

    let shock_l_ext = lk.and_then(|l| if l.nom_shock_l > 0.0 { Some(l.nom_shock_l) } else { None })
        .unwrap_or(sh.length_extended_mm);
    let stroke = sh.stroke_mm;

    let col3: Vec<f64> = (0..FULL_ROWS).map(|i| hypot(col1[i], col2[i])).collect();
    let mut col4 = vec![0.0_f64; FULL_ROWS];
    let mut col5 = vec![0.0_f64; FULL_ROWS];

    let mut shock_topped_index = 0usize;
    let mut shock_bottomed_index = FULL_ROWS - 2;
    let mut found_topped = false;
    let mut found_bottomed = false;

    for i in 0..FULL_ROWS {
        if !found_topped && col3[i] > shock_l_ext {
            shock_topped_index = if i > 0 { i - 1 } else { 0 };
            found_topped = true;
        }
        if !found_bottomed && col3[i] > shock_l_ext - stroke {
            shock_bottomed_index = if i > 0 { i - 1 } else { 0 };
            found_bottomed = true;
        }
    }
    if !found_topped { shock_topped_index = 0; }
    if !found_bottomed { shock_bottomed_index = FULL_ROWS - 2; }
    let shock_at_topped = col3[shock_topped_index];

    for j in 0..FULL_ROWS - 1 {
        col4[j] = col3[j + 1] - col3[j];
        col5[j] = shock_at_topped - col3[j];
    }

    let eff_sw_l = model.sw_l_mm;
    let swg_arm_ecc_ang = if eff_sw_l > 0.0 {
        deg((sw.offset / eff_sw_l).clamp(-1.0, 1.0).asin())
    } else {
        0.0
    };

    let col0: Vec<f64> = (0..FULL_ROWS).map(|i| (15.0 - i as f64 * 0.025) - swg_arm_ecc_ang).collect();

    let wheel_vert_at_topped = eff_sw_l * rad(col0[shock_topped_index]).sin();
    let col7: Vec<f64> = (0..FULL_ROWS).map(|i| eff_sw_l * rad(col0[i]).sin()).collect();
    let col8: Vec<f64> = (0..FULL_ROWS).map(|i| col7[i] - wheel_vert_at_topped).collect();

    let col4_computed = col4.clone();
    let mut col10 = vec![0.0_f64; FULL_ROWS];
    let mut col11 = vec![0.0_f64; FULL_ROWS];
    let mut col12 = vec![0.0_f64; FULL_ROWS];
    let mut col13 = vec![0.0_f64; FULL_ROWS];
    let mut col20 = vec![0.0_f64; FULL_ROWS];
    let mut col21 = vec![0.0_f64; FULL_ROWS];
    let mut col22 = vec![0.0_f64; FULL_ROWS];

    let rod_area_mm2 = PI * (sh.rod_dia_mm / 2.0).powi(2);
    let res_pressure = sh.res_pressure_bar;

    for m in 0..FULL_ROWS - 1 {
        col10[m] = (col8[m] + col8[m + 1]) / 2.0;
        let d_wheel = col8[m] - col8[m + 1];
        col11[m] = d_wheel;
        if d_wheel.abs() > 1e-12 {
            col12[m] = col4_computed[m] / d_wheel;
            col13[m] = 1.0 / col12[m];
        }

        let spring_compr = shock_l_ext - col3[m] + model.shock_preload_mm;
        let spring_force = spring_compr * sh.spring_rate_n_per_mm;

        let is_gas = sh.shock_type.as_deref()
            .map(|t| t.eq_ignore_ascii_case("GAS"))
            .unwrap_or(false);
        let gas_force = if is_gas && sh.res_vol_cc > 0.0 {
            let gf = res_pressure * (sh.res_vol_cc * 1000.0 / (sh.res_vol_cc * 1000.0 - rod_area_mm2 * col5[m])).powf(1.4);
            gf * 0.1 * rod_area_mm2
        } else {
            0.0
        };

        let topout_thresh = shock_l_ext - model.topout_l_mm;
        let topout_sag = if col3[m] > topout_thresh { col3[m] - topout_thresh } else { 0.0 };
        let topout_force = topout_sag * model.topout_rate_n_per_mm;

        let bump_thresh = shock_l_ext - stroke + sh.bump_ht_mm;
        let bump_sag = if col3[m] < bump_thresh { bump_thresh - col3[m] } else { 0.0 };
        let bump_force = bump_sag * sh.bump_rate_n_per_mm;

        let total_shock_force = spring_force + gas_force + bump_force - topout_force;
        col20[m] = total_shock_force * col12[m];
    }

    for n in 0..FULL_ROWS - 1 {
        col21[n] = col20[n] - col20[n + 1];
        if col11[n].abs() > 1e-12 {
            col22[n] = col21[n] / col11[n];
        }
    }
    let _ = col4_computed; // suppress unused warning

    let crop_start = shock_bottomed_index.saturating_sub(1);
    let crop_end = (shock_topped_index + 2).min(FULL_ROWS - 1);
    let rows: Vec<RearSuspRow> = (crop_start..=crop_end).map(|r| RearSuspRow {
        swingarm_angle_deg: col0[r],
        shock_pot_mm: col5[r],
        wheel_travel_mm: col8[r],
        motion_ratio_shock_per_wheel: col12[r],
        motion_ratio_wheel_per_shock: col13[r],
        wheel_force_n: col20[r],
        wheel_rate_n_per_mm: col22[r],
    }).collect();

    Some(RearSuspCurve {
        rows,
        eff_sw_l_mm: eff_sw_l,
        shock_l_ext_topped_mm: shock_at_topped,
    })
}

fn reversed_see_saw_sweep(
    fr_x: f64, fr_y: f64,
    swg_arm_link_angle_mod: f64, swg_arm_link_radius: f64,
    swg_arm_shock_angle_mod: f64, swg_arm_shock_radius: f64,
    anchor_linkarm: f64, anchor_shock: f64, shock_linkarm: f64, linkarm_l: f64,
    rocker_orientation: Option<&str>,
) -> (Vec<f64>, Vec<f64>) {
    const ROW_COUNT: usize = 1201;

    let rocker_incl_angle = compute_rocker_incl_angle(anchor_linkarm, anchor_shock, shock_linkarm, rocker_orientation);

    let mut col1 = vec![0.0_f64; ROW_COUNT];
    let mut col2 = vec![0.0_f64; ROW_COUNT];

    for i in 0..ROW_COUNT {
        let num2 = 15.0 - i as f64 * 0.025;

        let shock_ang = swg_arm_shock_angle_mod + num2;
        let num3 = rad(shock_ang).cos() * swg_arm_shock_radius;
        let num4 = rad(shock_ang).sin() * swg_arm_shock_radius;

        let link_ang = swg_arm_link_angle_mod + num2;
        let num5 = rad(link_ang).cos() * swg_arm_link_radius;
        let num6 = rad(link_ang).sin() * swg_arm_link_radius;

        let dx56 = num5 - fr_x;
        let dy56 = num6 - fr_y;
        let num7 = (dx56 * dx56 + dy56 * dy56).sqrt();
        if num7 < 1e-9 { continue; }

        let num8 = 180.0 - deg((num5 - fr_x).atan2(num6 - fr_y));

        let cos_arg = ((linkarm_l * linkarm_l - num7 * num7 - anchor_linkarm * anchor_linkarm)
            / (2.0 * num7 * anchor_linkarm))
            .clamp(-1.0, 1.0);
        let num9 = 180.0 - deg(cos_arg.acos());
        let num10 = 90.0 - (num9 - num8);

        let value = if rocker_orientation.map(|o| o.eq_ignore_ascii_case("DOWN")).unwrap_or(false) {
            let inner = 270.0 - num10;
            let num11 = 360.0 - rocker_incl_angle;
            inner - num11
        } else {
            -num10 + rocker_incl_angle - 90.0
        };

        let num12 = fr_x + rad(value).sin() * anchor_shock;
        let num13 = fr_y + rad(value).cos() * anchor_shock;

        col1[i] = num12 - num3;
        col2[i] = num13 - num4;
    }

    (col1, col2)
}

fn compute_rocker_incl_angle(anchor_linkarm: f64, anchor_shock: f64, shock_linkarm: f64, orientation: Option<&str>) -> f64 {
    let cos_val = ((anchor_linkarm * anchor_linkarm + anchor_shock * anchor_shock - shock_linkarm * shock_linkarm)
        / (2.0 * anchor_linkarm * anchor_shock))
        .clamp(-1.0, 1.0);
    let angle = deg(cos_val.acos());
    match orientation.map(|o| o.to_ascii_uppercase()).as_deref() {
        Some("DOWN") => 360.0 - angle,
        Some("UP") => angle,
        _ => 1.0,
    }
}

// ---------------------------------------------------------------------------
// Fork force curve builder
// ---------------------------------------------------------------------------

fn build_fork_curve(model: &ChassisModel) -> Option<ForkForceCurve> {
    let fork = model.fork.as_ref()?;
    model.frame.as_ref()?;

    if fork.travel_mm <= 0.0 || fork.tube_dia_mm <= 0.0 {
        return None;
    }

    const FORK_INCREMENT: f64 = 0.1;

    let tube_area = PI * (fork.tube_dia_mm / 2.0).powi(2);
    let rod_area = PI * (fork.rod_dia_mm / 2.0).powi(2);

    let piston_area = match fork.cartridge_type {
        CartridgeType::Gas | CartridgeType::GasAndSpring => tube_area - rod_area,
        _ => tube_area,
    };

    let res_piston_area = match fork.cartridge_type {
        CartridgeType::Gas | CartridgeType::GasAndSpring => {
            if fork.rod_thru_res_piston && fork.res_piston_dia_mm > 0.0 {
                PI * ((fork.res_piston_dia_mm / 2.0).powi(2) - (fork.res_piston_dia_inner_mm / 2.0).powi(2))
            } else if fork.res_piston_dia_mm > 0.0 {
                PI * (fork.res_piston_dia_mm / 2.0).powi(2)
            } else {
                0.0
            }
        }
        _ => 0.0,
    };

    let (air_vol_l, air_vol_r, air_spring_valid) = if fork.air_spring_mode == AirSpringMode::OilLevelTable {
        let l = interpolate_oil_level_table(&fork.oil_levels, &fork.air_volumes, fork.fork_l_oil_level_mm);
        let r = interpolate_oil_level_table(&fork.oil_levels, &fork.air_volumes, fork.fork_r_oil_level_mm);
        (l, r, l > 0.0 && r > 0.0)
    } else {
        (0.0, 0.0, false)
    };

    let rake_rad = model.frame.as_ref().map(|f| rad(f.head_angle_deg)).unwrap_or(0.0);

    let total_rows = ((fork.travel_mm / FORK_INCREMENT + 5.0 / FORK_INCREMENT) as usize) + 2;
    let mut raw_rows: Vec<ForkForceRow> = Vec::with_capacity(total_rows);
    let mut topped_index = 0usize;
    let mut prev_total = 0.0_f64;
    let mut first_row = true;

    let mut i = 0usize;
    loop {
        let comp = -5.0 + i as f64 * FORK_INCREMENT;
        if comp > fork.travel_mm {
            break;
        }

        if (comp - 0.0).abs() < FORK_INCREMENT / 2.0 && i > 0 {
            topped_index = raw_rows.len();
        }

        let comp_l = fork.fork_l_spr_pre_l_mm + comp;
        let comp_r = fork.fork_r_spr_pre_l_mm + comp;
        let spring_force = comp_l * fork.fork_l_spr_rate_n_per_mm + comp_r * fork.fork_r_spr_rate_n_per_mm;

        let air_force = if air_spring_valid
            && (air_vol_l * 1000.0 - comp * piston_area) > 0.0
            && (air_vol_r * 1000.0 - comp * piston_area) > 0.0
        {
            let press_l = 0.1 * (fork.p_bar * (air_vol_l * 1000.0 / (air_vol_l * 1000.0 - comp * piston_area)).powf(fork.kappa)) - 0.1013;
            let press_r = 0.1 * (fork.p_bar * (air_vol_r * 1000.0 / (air_vol_r * 1000.0 - comp * piston_area)).powf(fork.kappa)) - 0.1013;
            (press_l + press_r) * piston_area
        } else {
            0.0
        };

        let topout_l = if comp < fork.fork_l_top_l_mm {
            -(fork.fork_l_top_l_mm - comp) * fork.fork_l_top_rate_n_per_mm
        } else { 0.0 };
        let topout_r = if comp < fork.fork_r_top_l_mm {
            -(fork.fork_r_top_l_mm - comp) * fork.fork_r_top_rate_n_per_mm
        } else { 0.0 };

        let bump_force = if fork.bump_l_mm > 0.0 && fork.bump_rate_n_per_mm > 0.0
            && comp >= fork.travel_mm - fork.bump_l_mm
        {
            2.0 * (comp - (fork.travel_mm - fork.bump_l_mm)) * fork.bump_rate_n_per_mm
        } else { 0.0 };

        let topout_bump = topout_l + topout_r + bump_force;

        let res_force = match fork.cartridge_type {
            CartridgeType::Gas | CartridgeType::GasAndSpring => {
                let res_p32 = if fork.res_spring_chamber_vol_cc > 0.0 {
                    (fork.res_pressure_bar * 0.1 + 0.1013)
                        * (fork.res_spring_chamber_vol_cc * 1000.0
                            / (fork.res_spring_chamber_vol_cc * 1000.0 - rod_area * comp))
                            .powf(fork.kappa)
                        - 0.1013
                } else {
                    fork.res_pressure_bar * 0.1
                };
                2.0 * res_p32 * rod_area
            }
            CartridgeType::Spring | CartridgeType::SpringMech => {
                if res_piston_area > 0.0 {
                    let res_disp = comp * rod_area / res_piston_area;
                    let res_spring_force =
                        (res_disp + fork.res_spring_preload_mm) * fork.res_spring_rate_n_per_mm / res_piston_area;
                    2.0 * res_spring_force * rod_area
                } else { 0.0 }
            }
            _ => 0.0,
        };

        let total = spring_force + air_force + topout_bump + res_force;
        let fork_rate = if first_row { 0.0 } else { (total - prev_total) / FORK_INCREMENT };
        prev_total = total;
        first_row = false;

        raw_rows.push(ForkForceRow {
            fork_comp_mm: comp,
            fork_comp_wheel_mm: comp * rake_rad.cos(),
            spring_force_n: spring_force,
            air_force_n: air_force,
            topout_bump_force_n: topout_bump,
            reservoir_force_n: res_force,
            total_fork_force_n: total,
            fork_rate_n_per_mm: fork_rate,
        });

        i += 1;
    }

    raw_rows.truncate(i.min(raw_rows.len()));

    Some(ForkForceCurve {
        rows: raw_rows,
        topped_index,
        rake_rad,
    })
}

fn interpolate_oil_level_table(oil_levels: &[f64], air_volumes: &[f64], target: f64) -> f64 {
    if oil_levels.is_empty() || oil_levels.len() != air_volumes.len() {
        return 0.0;
    }
    let max_oil = oil_levels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if target >= oil_levels[0] && target <= max_oil {
        for i in 1..oil_levels.len() {
            if oil_levels[i] >= target {
                let span = oil_levels[i] - oil_levels[i - 1];
                let t = if span < 1e-12 { 0.0 } else { (target - oil_levels[i - 1]) / span };
                return air_volumes[i - 1] + t * (air_volumes[i] - air_volumes[i - 1]);
            }
        }
    }
    let n = oil_levels.len();
    if n >= 2 {
        let span = oil_levels[n - 1] - oil_levels[n - 2];
        let t = if span < 1e-12 { 0.0 } else { (target - oil_levels[n - 2]) / span };
        air_volumes[n - 2] + t * (air_volumes[n - 1] - air_volumes[n - 2])
    } else {
        air_volumes[0]
    }
}

// ---------------------------------------------------------------------------
// Rear curve lookups
// ---------------------------------------------------------------------------

fn lookup_closest(
    curve: &RearSuspCurve,
    rr_pot_mm: f64,
    sw_angle: &mut f64,
    wheel_travel: &mut f64,
    wheel_force: &mut f64,
    wheel_rate: &mut f64,
    mr_sw: &mut f64,
    mr_ws: &mut f64,
) {
    let rows = &curve.rows;
    if rows.is_empty() { return; }

    let mut best = 0usize;
    let mut best_diff = (rows[0].shock_pot_mm - rr_pot_mm).abs();
    for i in 1..rows.len() {
        let diff = (rows[i].shock_pot_mm - rr_pot_mm).abs();
        if diff < best_diff { best_diff = diff; best = i; }
    }
    let r = &rows[best];
    *sw_angle = r.swingarm_angle_deg;
    *wheel_travel = r.wheel_travel_mm;
    *wheel_force = r.wheel_force_n;
    *wheel_rate = r.wheel_rate_n_per_mm;
    *mr_sw = r.motion_ratio_shock_per_wheel;
    *mr_ws = r.motion_ratio_wheel_per_shock;
}

fn lookup_at_fr_pot(
    curve: &ForkForceCurve,
    fr_pot_mm: f64,
    inst_fork_angle_deg: f64,
    wheel_force: &mut f64,
    wheel_rate: &mut f64,
    fork_force: &mut f64,
    fork_rate: &mut f64,
    wheel_comp: &mut f64,
) {
    let rows = &curve.rows;
    if rows.is_empty() { return; }

    let mut idx = 0usize;
    for i in 0..rows.len() {
        if rows[i].fork_comp_mm >= fr_pot_mm { idx = i; break; }
        idx = i;
    }

    let row = &rows[idx];
    let cos_fork = rad(inst_fork_angle_deg).cos();
    let abs_cos = cos_fork.abs();
    *fork_force = row.total_fork_force_n;
    *fork_rate = row.fork_rate_n_per_mm;
    *wheel_comp = row.fork_comp_mm * cos_fork;

    if abs_cos < 1e-9 {
        *wheel_force = *fork_force;
        *wheel_rate = *fork_rate;
    } else {
        *wheel_force = *fork_force / cos_fork;
        *wheel_rate = *fork_rate / (cos_fork * cos_fork);
    }
}

// ---------------------------------------------------------------------------
// Math helpers
// ---------------------------------------------------------------------------

#[inline]
fn rad(deg: f64) -> f64 { deg * PI / 180.0 }
#[inline]
fn deg(rad: f64) -> f64 { rad * 180.0 / PI }
#[inline]
fn hypot(x: f64, y: f64) -> f64 { (x * x + y * y).sqrt() }
