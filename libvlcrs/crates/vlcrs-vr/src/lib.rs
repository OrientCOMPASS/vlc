//! `vlcrs-vr` — the spherical/VR projection core of libvlcrs.
//!
//! This crate is deliberately **platform free and allocation light** so that it
//! can be unit tested on the host and reused by any renderer.  It contains the
//! Rust port of the "decode → project" pipeline used by
//! [xl_player](https://github.com/xl-player-developers/xl_player)
//! (`xl_mesh_factory.c`, `xl_mat4.c`, `xl_model_ball.c`, `xl_model_rect.c`,
//! `xl_head_tracker/*`) extended with the pieces that upstream libvlc does not
//! provide:
//!
//! * a full **format matrix**: coverage `360° / 180°` × stereo layout
//!   `mono / side-by-side / top-bottom`, plus `auto` (metadata driven) and
//!   `planar` (forced flat 2D);
//! * **eye selection** for stereo layouts (official libvlc hard codes the left
//!   eye);
//! * **180° hemispherical** geometry with boundary *convergence* so that manual
//!   yaw can never leave the picture (black borders are not acceptable);
//! * in-place switching: the resolved projection is pure data, so changing the
//!   mode/eye never touches the playback pipeline and the current position is
//!   preserved;
//! * head tracking (Cardboard `OrientationEKF` port) and HUD read-outs
//!   (yaw/pitch/fov/…).
//!
//! Coordinate conventions (identical to the reference implementation):
//!
//! * right handed GL space, camera at the origin looking towards `-Z`, `+Y` up;
//! * a sphere vertex for longitude `lon` / latitude `lat` is
//!   `(cos(lat)·sin(lon), sin(lat), −cos(lat)·cos(lon))`, so `lon = 0`
//!   (`u = 0.5`) is dead centre of the picture at rest;
//! * texture coordinates are produced in *display space* (`v = 0` at the bottom
//!   of the picture, `v = 1` at the top) and are then transformed by the
//!   `SurfaceTexture` matrix at sample time, exactly like the reference.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod downmix;
pub mod ekf;
pub mod hud;
pub mod mat4;
pub mod mesh;
pub mod projection;
pub mod shaders;
pub mod view;

pub use hud::HudSnapshot;
pub use mat4::Mat4;
pub use mesh::Mesh;
pub use projection::{
    Coverage, Eye, Geometry, MediaHints, Pose, ProjectionMode, ProjectionSource,
    ResolvedProjection, StereoLayout, UvRect,
};
pub use view::{ViewLimits, ViewState, ViewUpdate, Viewport};

/// Value used when a quantity is unknown / not applicable.
pub const NONE: f32 = f32::NAN;

/// Degrees → radians.
#[inline]
pub fn deg2rad(d: f32) -> f32 {
    d * std::f32::consts::PI / 180.0
}

/// Radians → degrees.
#[inline]
pub fn rad2deg(r: f32) -> f32 {
    r * 180.0 / std::f32::consts::PI
}

/// Clamp `v` into `[lo, hi]` (NaN safe: NaN is mapped to `lo`).
#[inline]
pub fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_nan() || v <= lo {
        lo
    } else if v >= hi {
        hi
    } else {
        v
    }
}

/// Wrap an angle in degrees into `[-180, 180)`.
#[inline]
pub fn wrap180(mut deg: f32) -> f32 {
    if !deg.is_finite() {
        return 0.0;
    }
    deg %= 360.0;
    if deg >= 180.0 {
        deg -= 360.0;
    } else if deg < -180.0 {
        deg += 360.0;
    }
    deg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_is_symmetric() {
        assert_eq!(wrap180(180.0), -180.0);
        assert_eq!(wrap180(-180.0), -180.0);
        assert_eq!(wrap180(190.0), -170.0);
        assert_eq!(wrap180(-190.0), 170.0);
        assert_eq!(wrap180(0.0), 0.0);
        assert_eq!(wrap180(720.0), 0.0);
    }

    #[test]
    fn clamp_handles_nan() {
        assert_eq!(clampf(f32::NAN, 1.0, 2.0), 1.0);
        assert_eq!(clampf(5.0, 1.0, 2.0), 2.0);
    }
}
