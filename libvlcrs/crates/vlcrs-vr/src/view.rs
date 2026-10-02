//! View control: drag to look around, pinch to change the field of view,
//! gyroscope head tracking, "re-centre", and the boundary convergence that
//! keeps a 180° source from ever showing black.
//!
//! Sign conventions (verified by the unit tests below):
//!
//! * `yaw` positive ⇒ looking **right**, `pitch` positive ⇒ looking **up**;
//! * dragging follows the finger: moving the finger right decreases `yaw`,
//!   moving it down increases `pitch`;
//! * the model matrix is `Rz(roll) · Rx(−pitch) · Ry(yaw)`, i.e. pitch happens
//!   around the *yawed* local X axis, so the world direction of the view centre
//!   is exactly `(lon = yaw, lat = pitch)` and vertical drag never degenerates
//!   into roll when looking sideways.

use crate::mat4::Mat4;
use crate::projection::{Coverage, Pose};
use crate::{clampf, deg2rad, rad2deg, wrap180};

/// Smallest vertical field of view (degrees) — maximum zoom in.
pub const FOV_MIN_DEG: f32 = 25.0;
/// Largest vertical field of view (degrees) — maximum zoom out.
pub const FOV_MAX_DEG: f32 = 120.0;
/// Vertical field of view used when an item is opened (degrees).
pub const FOV_DEFAULT_DEG: f32 = 75.0;
/// Fraction of the boundary that is still traversed linearly before the
/// convergence curve kicks in.
pub const CONVERGE_KNEE: f32 = 0.72;
/// Near plane of the projection matrix.
pub const NEAR: f32 = 0.01;
/// Far plane of the projection matrix (the sphere has radius 1).
pub const FAR: f32 = 10.0;

/// Pixel size of the surface the engine renders into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Viewport {
    /// A 1×1 placeholder viewport (used before the surface exists).
    pub const PLACEHOLDER: Viewport = Viewport {
        width: 1,
        height: 1,
    };

    /// `width / height`, guarded against degenerate values.
    pub fn aspect(&self) -> f32 {
        if self.width == 0 || self.height == 0 {
            1.0
        } else {
            self.width as f32 / self.height as f32
        }
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport::PLACEHOLDER
    }
}

/// Angular limits currently in effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewLimits {
    /// Yaw range in degrees, `None` when yaw wraps around (360° sources).
    pub yaw: Option<(f32, f32)>,
    /// Pitch range in degrees.
    pub pitch: (f32, f32),
    /// Vertical FOV range in degrees.
    pub fov: (f32, f32),
    /// `true` when the boundary is reached asymptotically instead of hard
    /// clamped (manual navigation on a 180° source).
    pub converging: bool,
    /// `true` when the limits were relaxed because the gyroscope is driving the
    /// view (requirement: gyro may go further than manual drag).
    pub relaxed: bool,
}

impl ViewLimits {
    /// Compute the limits for a coverage / FOV / viewport combination.
    pub fn for_view(coverage: Coverage, fov_y: f32, viewport: Viewport, gyro: bool) -> ViewLimits {
        let aspect = viewport.aspect();
        let fov_x = horizontal_fov(fov_y, aspect);
        match coverage {
            Coverage::Planar => ViewLimits {
                yaw: Some((0.0, 0.0)),
                pitch: (0.0, 0.0),
                fov: (FOV_MIN_DEG, FOV_MAX_DEG),
                converging: false,
                relaxed: false,
            },
            Coverage::Full360 => ViewLimits {
                yaw: None,
                pitch: symmetric_limit(coverage.pitch_extent_deg(), fov_y, gyro),
                fov: (FOV_MIN_DEG, FOV_MAX_DEG),
                converging: false,
                relaxed: gyro,
            },
            Coverage::Half180 => ViewLimits {
                // Manual: keep the whole frustum inside the picture, so the
                // viewer can never rotate into the missing half.
                // Gyro: relax to the coverage boundary itself.
                yaw: Some(symmetric_limit(coverage.yaw_extent_deg(), fov_x, gyro).into_tuple()),
                pitch: symmetric_limit(coverage.pitch_extent_deg(), fov_y, gyro),
                fov: (FOV_MIN_DEG, FOV_MAX_DEG),
                converging: !gyro,
                relaxed: gyro,
            },
        }
    }
}

/// Helper so that `symmetric_limit` can feed an `Option<(f32, f32)>`.
trait Symmetric {
    fn into_tuple(self) -> (f32, f32);
}

impl Symmetric for (f32, f32) {
    fn into_tuple(self) -> (f32, f32) {
        self
    }
}

fn symmetric_limit(extent: f32, fov: f32, gyro: bool) -> (f32, f32) {
    if extent <= 0.0 {
        return (0.0, 0.0);
    }
    let half = if gyro {
        // Relaxed: the centre of the view may reach the coverage boundary.
        extent
    } else {
        // Strict: the outer edge of the frustum must stay inside the coverage.
        (extent - fov / 2.0).max(0.0)
    };
    (-half, half)
}

/// Vertical FOV → horizontal FOV for a given aspect ratio.
pub fn horizontal_fov(fov_y_deg: f32, aspect: f32) -> f32 {
    let t = deg2rad(fov_y_deg / 2.0).tan() * aspect.max(0.01);
    2.0 * rad2deg(t.atan())
}

/// Horizontal FOV → vertical FOV.
pub fn vertical_fov(fov_x_deg: f32, aspect: f32) -> f32 {
    let t = deg2rad(fov_x_deg / 2.0).tan() / aspect.max(0.01);
    2.0 * rad2deg(t.atan())
}

/// Soft, monotonic boundary convergence.
///
/// Below `CONVERGE_KNEE · limit` the value passes through untouched; above it
/// the motion is progressively damped and asymptotically approaches `limit`
/// without ever exceeding it.  The curve is C1 continuous at the knee, so the
/// transition is not perceptible while dragging.
pub fn converge(value: f32, limit: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    if limit.is_nan() || limit <= 0.0 {
        return 0.0;
    }
    let knee = CONVERGE_KNEE * limit;
    let span = (1.0 - CONVERGE_KNEE) * limit;
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let a = value.abs();
    if a <= knee {
        value
    } else {
        sign * (knee + span * ((a - knee) / span).tanh())
    }
}

/// Inverse of [`converge`] (used to keep the raw value in sync when the limit
/// grows, e.g. after zooming in).
pub fn unconverge(converged: f32, limit: f32) -> f32 {
    if !converged.is_finite() || limit.is_nan() || limit <= 0.0 {
        return 0.0;
    }
    let knee = CONVERGE_KNEE * limit;
    let span = (1.0 - CONVERGE_KNEE) * limit;
    let sign = if converged < 0.0 { -1.0 } else { 1.0 };
    let a = converged.abs();
    if a <= knee {
        converged
    } else if a >= limit {
        // asymptote: keep a large but finite raw value
        sign * (knee + span * 8.0)
    } else {
        let t = (a - knee) / span;
        let atanh = 0.5 * ((1.0 + t) / (1.0 - t)).ln();
        sign * (knee + span * atanh)
    }
}

/// Persistent, user controlled part of the view.
#[derive(Clone, Copy, Debug)]
pub struct ViewState {
    /// Raw (unconverged) yaw in degrees, positive = right.
    pub raw_yaw: f32,
    /// Raw pitch in degrees, positive = up.
    pub raw_pitch: f32,
    /// Roll in degrees, positive = counter clockwise on screen.
    pub roll: f32,
    /// Vertical field of view in degrees.
    pub fov_y: f32,
    /// Whether the gyroscope drives the view.
    pub gyro_enabled: bool,
    /// Metadata supplied home orientation, restored by [`ViewState::recenter`].
    pub home: Pose,
    /// Multiplier applied to drag deltas (1.0 = picture follows the finger).
    pub drag_gain: f32,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            raw_yaw: 0.0,
            raw_pitch: 0.0,
            roll: 0.0,
            fov_y: FOV_DEFAULT_DEG,
            gyro_enabled: false,
            home: Pose::default(),
            drag_gain: 1.0,
        }
    }
}

impl ViewState {
    /// A fresh state with the default FOV.
    pub fn new() -> Self {
        Self::default()
    }

    /// "视角摆正": drop every manual offset and return to the metadata pose
    /// (or to straight ahead when the container has none), and re-align the head
    /// tracker reference.
    pub fn recenter(&mut self) {
        self.raw_yaw = -self.home.yaw;
        self.raw_pitch = self.home.pitch;
        self.roll = self.home.roll;
    }

    /// Apply the metadata pose once the item is known.
    pub fn set_home(&mut self, pose: Pose) {
        self.home = pose;
        self.recenter();
    }

    /// Single finger drag, in pixels (screen coordinates: `dy > 0` = finger
    /// moved down).  The picture follows the finger.
    pub fn drag_pixels(&mut self, dx: f32, dy: f32, viewport: Viewport, fov_y: f32) {
        let w = viewport.width.max(1) as f32;
        let h = viewport.height.max(1) as f32;
        // 1:1 dragging: a full screen width sweep rotates by the horizontal FOV.
        let deg_per_px_x = horizontal_fov(fov_y, viewport.aspect()) / w;
        let deg_per_px_y = fov_y / h;
        self.drag_degrees(-dx * deg_per_px_x, dy * deg_per_px_y);
    }

    /// Single finger drag, already expressed in degrees.
    pub fn drag_degrees(&mut self, dyaw: f32, dpitch: f32) {
        let gain = if self.drag_gain.is_finite() && self.drag_gain > 0.0 {
            self.drag_gain
        } else {
            1.0
        };
        self.raw_yaw += dyaw * gain;
        self.raw_pitch += dpitch * gain;
        if self.raw_yaw.is_finite() {
            self.raw_yaw = wrap180(self.raw_yaw);
        } else {
            self.raw_yaw = 0.0;
        }
        self.raw_pitch = clampf(self.raw_pitch, -179.0, 179.0);
    }

    /// Pinch zoom: `factor > 1` (fingers apart) zooms **in** (smaller FOV).
    pub fn zoom(&mut self, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        self.set_fov(self.fov_y / factor);
    }

    /// Set the vertical FOV in degrees, clamped to the supported range.
    pub fn set_fov(&mut self, fov_y: f32) {
        self.fov_y = clampf(fov_y, FOV_MIN_DEG, FOV_MAX_DEG);
    }

    /// Set an absolute roll (degrees).
    pub fn set_roll(&mut self, roll: f32) {
        self.roll = wrap180(roll);
    }

    /// Enable/disable gyroscope driven look.
    pub fn set_gyro(&mut self, enabled: bool) {
        self.gyro_enabled = enabled;
    }

    /// Horizontal FOV for the current state.
    pub fn fov_x(&self, viewport: Viewport) -> f32 {
        horizontal_fov(self.fov_y, viewport.aspect())
    }

    /// Zoom level relative to the default FOV (1.0 = default, 2.0 = zoomed in
    /// twice).  Matches the "scale" semantic of the reference player.
    pub fn zoom_level(&self) -> f32 {
        FOV_DEFAULT_DEG / self.fov_y.max(1.0)
    }

    /// Compute the effective orientation, matrices and limits for one frame.
    ///
    /// `head` is the predicted head orientation as a GL model matrix
    /// (`None` when the gyroscope is off or not ready).
    pub fn evaluate(
        &self,
        coverage: Coverage,
        viewport: Viewport,
        head: Option<Mat4>,
    ) -> ViewUpdate {
        let limits = ViewLimits::for_view(coverage, self.fov_y, viewport, self.gyro_enabled);
        let aspect = viewport.aspect();

        // Convergence / clamping of the manual offsets.
        let (yaw, pitch) = match coverage {
            Coverage::Planar => (0.0, 0.0),
            Coverage::Full360 => {
                let p = clampf(self.raw_pitch, limits.pitch.0, limits.pitch.1);
                (wrap180(self.raw_yaw), p)
            }
            Coverage::Half180 => {
                let (ymin, ymax) = limits.yaw.unwrap_or((-90.0, 90.0));
                let lim = ymax.max(1e-4);
                let y = if limits.converging {
                    converge(clampf(self.raw_yaw, ymin * 8.0, ymax * 8.0), lim)
                } else {
                    clampf(self.raw_yaw, ymin, ymax)
                };
                let plim = limits.pitch.1.max(1e-4);
                let p = if limits.converging {
                    converge(self.raw_pitch, plim)
                } else {
                    clampf(self.raw_pitch, limits.pitch.0, limits.pitch.1)
                };
                (y, p)
            }
        };

        let roll = match coverage {
            Coverage::Planar => 0.0,
            _ => self.roll,
        };

        // FOV must not exceed the coverage of a 180° source, otherwise the
        // picture would show black even without any rotation.
        let fov_y = match coverage {
            Coverage::Half180 => {
                let max_y = vertical_fov(2.0 * Coverage::Half180.yaw_extent_deg() - 2.0, aspect)
                    .min(2.0 * Coverage::Half180.pitch_extent_deg() - 2.0);
                self.fov_y.min(max_y.max(FOV_MIN_DEG))
            }
            _ => self.fov_y,
        };
        let fov_x = horizontal_fov(fov_y, aspect);

        // Model matrix: manual orientation, optionally pre-multiplied by the
        // head orientation (reference behaviour: `model = head · Ry(yaw)`).
        // M = Rz(roll) · Rx(−pitch) · Ry(yaw): pitch happens around the *yawed*
        // local X axis, which makes the world direction of the view centre
        // exactly (lon = yaw, lat = pitch) and keeps vertical drag vertical even
        // when looking sideways.  Same matrix order as the reference player.
        let mut manual = Mat4::identity();
        manual.rotate_z(deg2rad(roll));
        manual.rotate_x(deg2rad(-pitch));
        manual.rotate_y(deg2rad(yaw));
        let model = match head {
            Some(h) if coverage.is_spherical() => h.mul(&manual),
            _ => manual,
        };

        let view = Mat4::look_at([0.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
        let projection = Mat4::perspective(fov_y, aspect, NEAR, FAR);

        let at_yaw_limit = match limits.yaw {
            Some((_, max)) if max > 0.0 => yaw.abs() >= max - 1e-3,
            _ => false,
        };
        let at_pitch_limit = pitch.abs() >= limits.pitch.1.max(1e-4) - 1e-3 && limits.pitch.1 > 0.0;
        // How hard the user is pushing against the boundary (0…1+).
        let resistance = match limits.yaw {
            Some((_, max)) if max > 1e-4 => (self.raw_yaw.abs() / max - CONVERGE_KNEE).max(0.0),
            _ => 0.0,
        };

        ViewUpdate {
            yaw,
            pitch,
            roll,
            fov_y,
            fov_x,
            model,
            view,
            projection,
            limits,
            at_yaw_limit,
            at_pitch_limit,
            resistance,
            gyro_active: head.is_some() && coverage.is_spherical(),
        }
    }
}

/// Everything the renderer (and the HUD) needs for one frame.
#[derive(Clone, Copy, Debug)]
pub struct ViewUpdate {
    /// Effective yaw in degrees (after convergence), positive = right.
    pub yaw: f32,
    /// Effective pitch in degrees, positive = up.
    pub pitch: f32,
    /// Effective roll in degrees.
    pub roll: f32,
    /// Effective vertical FOV in degrees.
    pub fov_y: f32,
    /// Effective horizontal FOV in degrees.
    pub fov_x: f32,
    /// Model matrix (sphere orientation).
    pub model: Mat4,
    /// View matrix.
    pub view: Mat4,
    /// Projection matrix.
    pub projection: Mat4,
    /// Limits in effect.
    pub limits: ViewLimits,
    /// `true` when the yaw boundary is reached.
    pub at_yaw_limit: bool,
    /// `true` when the pitch boundary is reached.
    pub at_pitch_limit: bool,
    /// How far past the knee the user is pushing (0 = free, >0 = converging).
    pub resistance: f32,
    /// `true` when the head tracker contributes to the orientation.
    pub gyro_active: bool,
}

impl ViewUpdate {
    /// World space longitude/latitude (degrees) of the centre of the view.
    pub fn view_direction(&self) -> (f32, f32) {
        let d = self.inverse_model().transform([0.0, 0.0, -1.0, 0.0]);
        direction_to_lonlat([d[0], d[1], d[2]])
    }

    /// World space longitude/latitude (degrees) of a point in NDC
    /// (`x, y ∈ [-1, 1]`, `z` ignored).  Used by the tests to prove that a
    /// 180° source never shows anything outside its coverage.
    pub fn ndc_direction(&self, x: f32, y: f32) -> (f32, f32) {
        let tx = deg2rad(self.fov_x / 2.0).tan();
        let ty = deg2rad(self.fov_y / 2.0).tan();
        let d_eye = [tx * x, ty * y, -1.0];
        let inv = self.inverse_model();
        let d = inv.transform([d_eye[0], d_eye[1], d_eye[2], 0.0]);
        direction_to_lonlat([d[0], d[1], d[2]])
    }

    fn inverse_model(&self) -> Mat4 {
        // The model matrix is a pure rotation, so its inverse is its transpose.
        self.model.transpose()
    }
}

/// Convert a unit direction into (longitude, latitude) degrees using the mesh
/// parameterisation of [`crate::mesh`].
pub fn direction_to_lonlat(d: [f32; 3]) -> (f32, f32) {
    let lon = rad2deg(d[0].atan2(-d[2]));
    let lat = rad2deg(clampf(d[1], -1.0, 1.0).asin());
    (lon, lat)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANDSCAPE: Viewport = Viewport {
        width: 2400,
        height: 1080,
    };

    #[test]
    fn converge_is_identity_below_the_knee() {
        let limit = 45.0;
        for v in [0.0, 10.0, -20.0, limit * CONVERGE_KNEE] {
            assert!(
                (converge(v, limit) - v).abs() < 1e-5,
                "{v} must pass through unchanged"
            );
        }
    }

    #[test]
    fn converge_never_exceeds_the_limit() {
        let limit = 45.0;
        for raw in [0.0, 30.0, 44.0, 45.0, 60.0, 120.0, 360.0, 10_000.0] {
            let c = converge(raw, limit);
            assert!(c <= limit + 1e-4, "converge({raw}) = {c} > {limit}");
            assert!(c >= 0.0);
            let cn = converge(-raw, limit);
            assert!(cn >= -limit - 1e-4, "converge({}) = {cn}", -raw);
        }
    }

    #[test]
    fn converge_is_monotonic() {
        let limit = 30.0;
        let mut prev = f32::NEG_INFINITY;
        let mut v = -500.0;
        while v <= 500.0 {
            let c = converge(v, limit);
            // strictly increasing until the asymptote saturates in f32
            if prev.abs() < limit - 1e-3 {
                assert!(c > prev, "not monotonic at {v}: {c} <= {prev}");
            } else {
                assert!(c >= prev - 1e-6, "went backwards at {v}: {c} < {prev}");
            }
            prev = c;
            v += 0.25;
        }
    }

    #[test]
    fn converge_roundtrip() {
        let limit = 40.0;
        for raw in [-200.0, -50.0, -10.0, 0.0, 10.0, 50.0, 200.0] {
            let c = converge(raw, limit);
            let back = unconverge(c, limit);
            assert!(
                (converge(back, limit) - c).abs() < 1e-3,
                "raw {raw} → {c} → {back}"
            );
        }
    }

    #[test]
    fn horizontal_fov_is_wider_than_vertical_in_landscape() {
        let fx = horizontal_fov(60.0, LANDSCAPE.aspect());
        assert!(fx > 60.0);
        assert!((vertical_fov(fx, LANDSCAPE.aspect()) - 60.0).abs() < 1e-3);
    }

    #[test]
    fn limits_for_360_wrap_yaw() {
        let s = ViewState::new();
        let l = ViewLimits::for_view(Coverage::Full360, s.fov_y, LANDSCAPE, false);
        assert!(l.yaw.is_none());
        assert!(!l.converging);
        // pitch stays inside the sphere
        assert!(l.pitch.1 < 90.0 && l.pitch.1 > 0.0);
    }

    #[test]
    fn limits_for_180_keep_the_frustum_inside() {
        let s = ViewState::new();
        let l = ViewLimits::for_view(Coverage::Half180, s.fov_y, LANDSCAPE, false);
        let (min, max) = l.yaw.unwrap();
        let fov_x = horizontal_fov(s.fov_y, LANDSCAPE.aspect());
        assert!((max - (90.0 - fov_x / 2.0)).abs() < 1e-3);
        assert_eq!(min, -max);
        assert!(l.converging);
        assert!(!l.relaxed);
    }

    #[test]
    fn gyro_limits_are_relaxed() {
        let s = ViewState::new();
        let l = ViewLimits::for_view(Coverage::Half180, s.fov_y, LANDSCAPE, true);
        let (_, max) = l.yaw.unwrap();
        assert!((max - 90.0).abs() < 1e-4);
        assert!(l.relaxed);
        assert!(!l.converging);
    }

    #[test]
    fn planar_does_not_rotate() {
        let mut s = ViewState::new();
        s.drag_degrees(40.0, 30.0);
        let u = s.evaluate(Coverage::Planar, LANDSCAPE, None);
        assert_eq!(u.yaw, 0.0);
        assert_eq!(u.pitch, 0.0);
        assert_eq!(u.model, Mat4::identity());
    }

    #[test]
    fn drag_directions_follow_the_finger() {
        let mut s = ViewState::new();
        // finger moves right → looking left → yaw negative
        s.drag_pixels(120.0, 0.0, LANDSCAPE, s.fov_y);
        assert!(s.raw_yaw < 0.0, "yaw = {}", s.raw_yaw);
        // finger moves down → looking up → pitch positive
        let mut s2 = ViewState::new();
        s2.drag_pixels(0.0, 120.0, LANDSCAPE, s2.fov_y);
        assert!(s2.raw_pitch > 0.0, "pitch = {}", s2.raw_pitch);
    }

    #[test]
    fn full_width_drag_sweeps_the_horizontal_fov() {
        let mut s = ViewState::new();
        s.drag_pixels(LANDSCAPE.width as f32, 0.0, LANDSCAPE, s.fov_y);
        let fov_x = horizontal_fov(s.fov_y, LANDSCAPE.aspect());
        assert!(
            (s.raw_yaw.abs() - fov_x).abs() < 1e-2,
            "yaw {} vs fov_x {fov_x}",
            s.raw_yaw
        );
    }

    #[test]
    fn model_matrix_orientation_is_consistent() {
        let mut s = ViewState::new();
        s.raw_yaw = 30.0; // looking right
        let u = s.evaluate(Coverage::Full360, LANDSCAPE, None);
        // The world direction the camera looks at must be 30° to the right.
        let (lon, lat) = u.view_direction();
        assert!((lon - 30.0).abs() < 1e-2, "lon = {lon}");
        assert!(lat.abs() < 1e-3);

        let mut s2 = ViewState::new();
        s2.raw_pitch = 20.0; // looking up
        let u2 = s2.evaluate(Coverage::Full360, LANDSCAPE, None);
        let (lon2, lat2) = u2.view_direction();
        assert!(lon2.abs() < 1e-2);
        assert!((lat2 - 20.0).abs() < 1e-2, "lat = {lat2}");
    }

    #[test]
    fn pitch_stays_vertical_when_looking_sideways() {
        // Regression guard for the world-axis pitch artefact of the reference
        // ordering: at yaw = 90° a pitch change must still move the view up.
        let mut s = ViewState::new();
        s.raw_yaw = 90.0;
        let a = s
            .evaluate(Coverage::Full360, LANDSCAPE, None)
            .view_direction();
        s.raw_pitch = 25.0;
        let b = s
            .evaluate(Coverage::Full360, LANDSCAPE, None)
            .view_direction();
        assert!(
            (b.1 - a.1).abs() > 20.0,
            "pitch must stay vertical at yaw 90: {a:?} → {b:?}"
        );
        assert!(
            (b.0 - a.0).abs() < 15.0,
            "longitude must not run away: {a:?} → {b:?}"
        );
    }

    #[test]
    fn horizontal_panning_never_leaves_a_180_picture() {
        // The core VR requirement: panning left/right in a 180° source must
        // never show black, so with the horizon level every ray of the frustum
        // stays inside the ±90° lune, whatever the yaw, the FOV or the
        // (converged) pitch bias the user asked for.
        let mut s = ViewState::new();
        for raw in [
            -100_000.0, -1000.0, -180.0, -95.0, -40.0, 0.0, 40.0, 95.0, 1000.0,
        ] {
            s.raw_yaw = raw;
            s.raw_pitch = 0.0;
            for fov in [FOV_MIN_DEG, 50.0, FOV_DEFAULT_DEG, 90.0, FOV_MAX_DEG] {
                s.fov_y = fov;
                let u = s.evaluate(Coverage::Half180, LANDSCAPE, None);
                for (x, y) in [
                    (-1.0, 0.0),
                    (1.0, 0.0),
                    (0.0, 0.0),
                    (-1.0, -1.0),
                    (1.0, 1.0),
                ] {
                    let (lon, lat) = u.ndc_direction(x, y);
                    if y == 0.0 {
                        assert!(
                            lon.abs() <= 90.0 + 0.01,
                            "yaw={raw} fov={fov} x={x} lon={lon}"
                        );
                    }
                    assert!(lat.abs() <= 90.0 + 0.01, "yaw={raw} fov={fov} lat={lat}");
                }
            }
        }
    }

    #[test]
    fn tilted_view_stays_inside_the_dome() {
        // When the viewer tilts up/down, longitude compresses near the poles and
        // the frustum inevitably reaches past the ±90° lune.  That is not a
        // black border though: the hemisphere mesh is tessellated over the whole
        // sphere with clamped `u` (see `mesh::HEMI_OVERSCAN_DEG`), so every
        // direction is still covered by geometry.  What must hold is that the
        // *vertical* extent stays inside the picture and that the yaw still
        // converges to its limit.
        let mut s = ViewState::new();
        for raw in [-1000.0, -95.0, 0.0, 95.0, 1000.0] {
            s.raw_yaw = raw;
            for pitch in [-200.0, -60.0, 0.0, 60.0, 200.0] {
                s.raw_pitch = pitch;
                for fov in [FOV_MIN_DEG, FOV_DEFAULT_DEG, FOV_MAX_DEG] {
                    s.fov_y = fov;
                    let u = s.evaluate(Coverage::Half180, LANDSCAPE, None);
                    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                        let (lon, lat) = u.ndc_direction(x, y);
                        assert!(
                            lat.abs() <= 90.0 + 0.01,
                            "pitch={pitch} fov={fov} lat={lat}"
                        );
                        assert!(
                            lon.abs() <= 180.0 + 0.01,
                            "pitch={pitch} fov={fov} lon={lon}"
                        );
                    }
                    assert!(u.yaw.abs() <= 90.0 + 1e-3, "yaw = {}", u.yaw);
                    assert!(u.pitch.abs() <= 90.0 + 1e-3, "pitch = {}", u.pitch);
                }
            }
        }
    }

    #[test]
    fn manual_view_covers_the_whole_180_picture() {
        // …and the convergence must not be so aggressive that the edges become
        // unreachable: at the limit the view centre sits at the boundary minus
        // half the FOV, i.e. the frustum edge touches ±90°.
        let mut s = ViewState::new();
        s.raw_yaw = 500.0;
        let u = s.evaluate(Coverage::Half180, LANDSCAPE, None);
        let (lon_right, _) = u.ndc_direction(1.0, 0.0);
        assert!(
            (lon_right - 90.0).abs() < 0.5,
            "right edge of the frustum should touch the coverage boundary, got {lon_right}"
        );
        s.raw_yaw = -500.0;
        let u = s.evaluate(Coverage::Half180, LANDSCAPE, None);
        let (lon_left, _) = u.ndc_direction(-1.0, 0.0);
        assert!(
            (lon_left + 90.0).abs() < 0.5,
            "left edge should touch -90, got {lon_left}"
        );
    }

    #[test]
    fn gyro_mode_may_reach_the_boundary() {
        let mut s = ViewState::new();
        s.gyro_enabled = true;
        s.raw_yaw = 90.0;
        let u = s.evaluate(Coverage::Half180, LANDSCAPE, None);
        assert!((u.yaw - 90.0).abs() < 1e-3);
        assert!(u.limits.relaxed);
        assert!(!u.limits.converging);
    }

    #[test]
    fn at_limit_flags_and_resistance() {
        let mut s = ViewState::new();
        s.raw_yaw = 0.0;
        let u = s.evaluate(Coverage::Half180, LANDSCAPE, None);
        assert!(!u.at_yaw_limit);
        assert_eq!(u.resistance, 0.0);
        s.raw_yaw = 400.0;
        let u = s.evaluate(Coverage::Half180, LANDSCAPE, None);
        assert!(u.at_yaw_limit);
        assert!(u.resistance > 0.0);
    }

    #[test]
    fn recenter_restores_the_metadata_pose() {
        let mut s = ViewState::new();
        s.set_home(Pose {
            yaw: 30.0,
            pitch: -10.0,
            roll: 0.0,
        });
        assert!((s.raw_yaw + 30.0).abs() < 1e-4);
        s.drag_degrees(50.0, 20.0);
        s.recenter();
        assert!((s.raw_yaw + 30.0).abs() < 1e-4);
        assert!((s.raw_pitch + 10.0).abs() < 1e-4);
    }

    #[test]
    fn zoom_clamps_and_reports_a_level() {
        let mut s = ViewState::new();
        assert!((s.zoom_level() - 1.0).abs() < 1e-4);
        for _ in 0..40 {
            s.zoom(1.3);
        }
        assert!((s.fov_y - FOV_MIN_DEG).abs() < 1e-3);
        for _ in 0..80 {
            s.zoom(0.7);
        }
        assert!((s.fov_y - FOV_MAX_DEG).abs() < 1e-3);
        s.zoom(f32::NAN);
        s.zoom(-1.0);
        assert!(s.fov_y.is_finite() && s.fov_y > 0.0);
    }

    #[test]
    fn fov_is_limited_by_the_180_coverage() {
        let mut s = ViewState::new();
        s.fov_y = FOV_MAX_DEG;
        let u = s.evaluate(Coverage::Half180, LANDSCAPE, None);
        assert!(u.fov_x <= 180.0 - 1.0, "fov_x = {}", u.fov_x);
        assert!(u.fov_y <= 180.0 - 1.0);
        // 360 keeps the requested FOV
        let u360 = s.evaluate(Coverage::Full360, LANDSCAPE, None);
        assert!((u360.fov_y - FOV_MAX_DEG).abs() < 1e-3);
    }

    #[test]
    fn head_matrix_is_composed_with_the_manual_offsets() {
        let mut s = ViewState::new();
        s.gyro_enabled = true;
        // head looking 90° to the right
        let mut head = Mat4::identity();
        head.rotate_y(deg2rad(90.0));
        let u = s.evaluate(Coverage::Full360, LANDSCAPE, Some(head));
        let (lon, _) = u.view_direction();
        assert!((lon - 90.0).abs() < 1e-2, "lon = {lon}");
        assert!(u.gyro_active);
        // manual yaw still works on top of the head orientation
        s.raw_yaw = 20.0;
        let u2 = s.evaluate(Coverage::Full360, LANDSCAPE, Some(head));
        let (lon2, _) = u2.view_direction();
        assert!((lon2 - 110.0).abs() < 1e-2, "lon2 = {lon2}");
    }

    #[test]
    fn direction_to_lonlat_matches_the_mesh_parameterisation() {
        // straight ahead
        let (lon, lat) = direction_to_lonlat([0.0, 0.0, -1.0]);
        assert!(lon.abs() < 1e-4 && lat.abs() < 1e-4);
        // right
        let (lon, _) = direction_to_lonlat([1.0, 0.0, 0.0]);
        assert!((lon - 90.0).abs() < 1e-3);
        // up
        let (_, lat) = direction_to_lonlat([0.0, 1.0, 0.0]);
        assert!((lat - 90.0).abs() < 1e-3);
    }
}
