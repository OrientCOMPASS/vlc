//! HUD read-outs (requirement: show yaw / pitch / FOV while looking around).
//!
//! [`HudSnapshot`] is a plain data structure filled once per frame by the
//! engine; it can be rendered by the app, formatted with [`HudSnapshot::lines`]
//! or shipped across JNI as a compact float array
//! ([`HudSnapshot::to_floats`] / [`HudSnapshot::from_floats`]).

/// Number of floats in the compact JNI representation.
pub const HUD_FLOAT_COUNT: usize = 20;

/// Field indices of the compact float representation. Stable ABI.
pub mod field {
    /// Effective yaw in degrees.
    pub const YAW: usize = 0;
    /// Effective pitch in degrees.
    pub const PITCH: usize = 1;
    /// Effective roll in degrees.
    pub const ROLL: usize = 2;
    /// Vertical FOV in degrees.
    pub const FOV_Y: usize = 3;
    /// Horizontal FOV in degrees.
    pub const FOV_X: usize = 4;
    /// Projection mode id (`ProjectionMode::as_i32`).
    pub const MODE: usize = 5;
    /// Coverage id (0 planar, 1 360, 2 180).
    pub const COVERAGE: usize = 6;
    /// Stereo layout id (0 mono, 1 SBS, 2 TB).
    pub const LAYOUT: usize = 7;
    /// Eye id (0 left, 1 right).
    pub const EYE: usize = 8;
    /// Projection source id (0 forced, 1 metadata, 2 fallback).
    pub const SOURCE: usize = 9;
    /// Yaw boundary reached (1/0).
    pub const AT_YAW_LIMIT: usize = 10;
    /// Pitch boundary reached (1/0).
    pub const AT_PITCH_LIMIT: usize = 11;
    /// Boundary convergence active (1/0).
    pub const CONVERGING: usize = 12;
    /// How hard the user pushes against the boundary.
    pub const RESISTANCE: usize = 13;
    /// Gyroscope enabled (1/0).
    pub const GYRO: usize = 14;
    /// Head tracker aligned (1/0).
    pub const GYRO_READY: usize = 15;
    /// Zoom level relative to the default FOV.
    pub const ZOOM: usize = 16;
    /// Playback position in seconds.
    pub const POSITION_S: usize = 17;
    /// Duration in seconds.
    pub const DURATION_S: usize = 18;
    /// Rendered frames per second.
    pub const FPS: usize = 19;
}

/// One HUD sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudSnapshot {
    /// Effective yaw (degrees, positive right).
    pub yaw: f32,
    /// Effective pitch (degrees, positive up).
    pub pitch: f32,
    /// Effective roll (degrees).
    pub roll: f32,
    /// Vertical FOV (degrees).
    pub fov_y: f32,
    /// Horizontal FOV (degrees).
    pub fov_x: f32,
    /// Zoom level (1.0 = default).
    pub zoom: f32,
    /// Projection mode label.
    pub mode: &'static str,
    /// Coverage label ("2D" / "360°" / "180°").
    pub coverage: &'static str,
    /// Layout label ("mono" / "SBS" / "TB").
    pub layout: &'static str,
    /// Eye label ("L" / "R").
    pub eye: &'static str,
    /// Where the projection came from ("forced"/"metadata"/"fallback").
    pub source: &'static str,
    /// Yaw boundary reached.
    pub at_yaw_limit: bool,
    /// Pitch boundary reached.
    pub at_pitch_limit: bool,
    /// Convergence active (manual navigation on 180°).
    pub converging: bool,
    /// Boundary pressure, 0 = free.
    pub resistance: f32,
    /// Gyroscope enabled.
    pub gyro: bool,
    /// Head tracker has a gravity reference.
    pub gyro_ready: bool,
    /// Playback position in milliseconds.
    pub position_ms: i64,
    /// Duration in milliseconds (0 = unknown).
    pub duration_ms: i64,
    /// Coded video width.
    pub video_width: u32,
    /// Coded video height.
    pub video_height: u32,
    /// Display rotation of the picture.
    pub rotation: i32,
    /// Rendered frames per second.
    pub fps: f32,
    /// Decoded frames since start.
    pub decoded_frames: u64,
    /// Dropped (late) frames since start.
    pub dropped_frames: u64,
    /// Buffered media ahead of the clock, in milliseconds.
    pub buffered_ms: i64,
    /// Playback state label.
    pub state: &'static str,
    /// Surface width in pixels.
    pub viewport_w: u32,
    /// Surface height in pixels.
    pub viewport_h: u32,
    /// Numeric ids `[mode, coverage, layout, eye, source]`, see [`field`].
    pub ids: [i32; 5],
}

impl Default for HudSnapshot {
    fn default() -> Self {
        HudSnapshot {
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            fov_y: 0.0,
            fov_x: 0.0,
            zoom: 1.0,
            mode: "Auto",
            coverage: "2D",
            layout: "mono",
            eye: "L",
            source: "fallback",
            at_yaw_limit: false,
            at_pitch_limit: false,
            converging: false,
            resistance: 0.0,
            gyro: false,
            gyro_ready: false,
            position_ms: 0,
            duration_ms: 0,
            video_width: 0,
            video_height: 0,
            rotation: 0,
            fps: 0.0,
            decoded_frames: 0,
            dropped_frames: 0,
            buffered_ms: 0,
            state: "Idle",
            viewport_w: 0,
            viewport_h: 0,
            ids: [0; 5],
        }
    }
}

impl HudSnapshot {
    /// Signed angle formatting helper: `+12.3°`.
    pub fn fmt_angle(v: f32) -> String {
        format!("{:+.1}°", v)
    }

    /// `mm:ss` (or `h:mm:ss`) formatting helper.
    pub fn fmt_time(ms: i64) -> String {
        let neg = ms < 0;
        let total = ms.unsigned_abs() / 1000;
        let h = total / 3600;
        let m = (total % 3600) / 60;
        let s = total % 60;
        let body = if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m:02}:{s:02}")
        };
        if neg {
            format!("-{body}")
        } else {
            body
        }
    }

    /// The lines to show in an overlay.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(5);
        out.push(format!(
            "YAW {}   PITCH {}   ROLL {}",
            Self::fmt_angle(self.yaw),
            Self::fmt_angle(self.pitch),
            Self::fmt_angle(self.roll),
        ));
        out.push(format!(
            "FOV {:.0}°×{:.0}°   ZOOM {:.2}×",
            self.fov_y, self.fov_x, self.zoom
        ));
        let mut boundary = Vec::new();
        if self.at_yaw_limit {
            boundary.push("YAW LIMIT");
        }
        if self.at_pitch_limit {
            boundary.push("PITCH LIMIT");
        }
        if self.converging && boundary.is_empty() && self.resistance > 0.0 {
            boundary.push("CONVERGING");
        }
        let eye = if self.layout == "mono" {
            String::new()
        } else {
            format!(" eye {}", self.eye)
        };
        let src = format!("  [{}]", self.source);
        let bound = if boundary.is_empty() {
            String::new()
        } else {
            format!("  {}", boundary.join(" "))
        };
        out.push(format!(
            "{} {} {}{}{}{}",
            self.mode, self.coverage, self.layout, eye, src, bound
        ));
        out.push(format!(
            "GYRO {}{}",
            if self.gyro { "ON" } else { "OFF" },
            if self.gyro && self.gyro_ready {
                " (ready)"
            } else if self.gyro {
                " (aligning…)"
            } else {
                ""
            },
        ));
        out.push(format!(
            "{} / {}   {:.1} fps   {}/{} frames   {}×{}   buf {} ms   {}",
            Self::fmt_time(self.position_ms),
            if self.duration_ms > 0 {
                Self::fmt_time(self.duration_ms)
            } else {
                "--:--".to_string()
            },
            self.fps,
            self.decoded_frames,
            self.decoded_frames + self.dropped_frames,
            self.video_width,
            self.video_height,
            self.buffered_ms,
            self.state,
        ));
        out
    }

    /// Single line variant for logs.
    pub fn compact(&self) -> String {
        format!(
            "yaw={} pitch={} fov={:.0}x{:.0} {} {}/{} src={} limit={}{} gyro={} pos={} fps={:.1} drop={}",
            self.yaw,
            self.pitch,
            self.fov_y,
            self.fov_x,
            self.mode,
            self.coverage,
            self.layout,
            self.source,
            self.at_yaw_limit,
            self.at_pitch_limit,
            self.gyro,
            Self::fmt_time(self.position_ms),
            self.fps,
            self.dropped_frames,
        )
    }

    /// Compact float transport for JNI.
    pub fn to_floats(&self) -> [f32; HUD_FLOAT_COUNT] {
        let mut f = [0.0f32; HUD_FLOAT_COUNT];
        f[field::YAW] = self.yaw;
        f[field::PITCH] = self.pitch;
        f[field::ROLL] = self.roll;
        f[field::FOV_Y] = self.fov_y;
        f[field::FOV_X] = self.fov_x;
        f[field::MODE] = self.ids[0] as f32;
        f[field::COVERAGE] = self.ids[1] as f32;
        f[field::LAYOUT] = self.ids[2] as f32;
        f[field::EYE] = self.ids[3] as f32;
        f[field::SOURCE] = self.ids[4] as f32;
        f[field::AT_YAW_LIMIT] = bool_to_f32(self.at_yaw_limit);
        f[field::AT_PITCH_LIMIT] = bool_to_f32(self.at_pitch_limit);
        f[field::CONVERGING] = bool_to_f32(self.converging);
        f[field::RESISTANCE] = self.resistance;
        f[field::GYRO] = bool_to_f32(self.gyro);
        f[field::GYRO_READY] = bool_to_f32(self.gyro_ready);
        f[field::ZOOM] = self.zoom;
        f[field::POSITION_S] = self.position_ms as f32 / 1000.0;
        f[field::DURATION_S] = self.duration_ms as f32 / 1000.0;
        f[field::FPS] = self.fps;
        f
    }

    /// Rebuild the numeric part of a snapshot from [`HudSnapshot::to_floats`].
    pub fn from_floats(f: &[f32]) -> HudSnapshot {
        let mut s = HudSnapshot::default();
        if f.len() < HUD_FLOAT_COUNT {
            return s;
        }
        s.yaw = f[field::YAW];
        s.pitch = f[field::PITCH];
        s.roll = f[field::ROLL];
        s.fov_y = f[field::FOV_Y];
        s.fov_x = f[field::FOV_X];
        s.ids[0] = f[field::MODE] as i32;
        s.ids[1] = f[field::COVERAGE] as i32;
        s.ids[2] = f[field::LAYOUT] as i32;
        s.ids[3] = f[field::EYE] as i32;
        s.ids[4] = f[field::SOURCE] as i32;
        s.at_yaw_limit = f32_to_bool(f[field::AT_YAW_LIMIT]);
        s.at_pitch_limit = f32_to_bool(f[field::AT_PITCH_LIMIT]);
        s.converging = f32_to_bool(f[field::CONVERGING]);
        s.resistance = f[field::RESISTANCE];
        s.gyro = f32_to_bool(f[field::GYRO]);
        s.gyro_ready = f32_to_bool(f[field::GYRO_READY]);
        s.zoom = f[field::ZOOM];
        s.position_ms = (f[field::POSITION_S] * 1000.0) as i64;
        s.duration_ms = (f[field::DURATION_S] * 1000.0) as i64;
        s.fps = f[field::FPS];
        s
    }
}

fn bool_to_f32(b: bool) -> f32 {
    if b {
        1.0
    } else {
        0.0
    }
}

fn f32_to_bool(v: f32) -> bool {
    v > 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HudSnapshot {
        HudSnapshot {
            yaw: 12.34,
            pitch: -5.0,
            roll: 0.0,
            fov_y: 75.0,
            fov_x: 133.4,
            zoom: 1.0,
            mode: "180 SBS",
            coverage: "180°",
            layout: "SBS",
            eye: "R",
            source: "forced",
            at_yaw_limit: false,
            at_pitch_limit: false,
            converging: true,
            resistance: 0.25,
            gyro: true,
            gyro_ready: true,
            position_ms: 65_000,
            duration_ms: 3_725_000,
            video_width: 3840,
            video_height: 1920,
            rotation: 0,
            fps: 59.94,
            decoded_frames: 1200,
            dropped_frames: 3,
            buffered_ms: 800,
            state: "Playing",
            viewport_w: 2400,
            viewport_h: 1080,
            ids: [6, 2, 1, 1, 0],
        }
    }

    #[test]
    fn lines_cover_the_required_readouts() {
        let s = sample();
        let lines = s.lines();
        let joined = lines.join("\n");
        // requirement 6: yaw / pitch / fov must be visible
        assert!(joined.contains("YAW +12.3°"), "{joined}");
        assert!(joined.contains("PITCH -5.0°"), "{joined}");
        assert!(joined.contains("FOV 75°×133°"), "{joined}");
        assert!(joined.contains("180 SBS"), "{joined}");
        assert!(joined.contains("eye R"), "{joined}");
        assert!(joined.contains("CONVERGING"), "{joined}");
        assert!(joined.contains("GYRO ON (ready)"), "{joined}");
        assert!(joined.contains("01:05 / 1:02:05"), "{joined}");
        assert!(joined.contains("3840×1920"), "{joined}");
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn boundary_flags_are_reported() {
        let mut s = sample();
        s.at_yaw_limit = true;
        s.at_pitch_limit = true;
        let joined = s.lines().join("\n");
        assert!(joined.contains("YAW LIMIT"));
        assert!(joined.contains("PITCH LIMIT"));
    }

    #[test]
    fn time_formatting() {
        assert_eq!(HudSnapshot::fmt_time(0), "00:00");
        assert_eq!(HudSnapshot::fmt_time(65_000), "01:05");
        assert_eq!(HudSnapshot::fmt_time(3_725_000), "1:02:05");
        assert_eq!(HudSnapshot::fmt_time(-1500), "-00:01");
    }

    #[test]
    fn float_transport_roundtrip() {
        let s = sample();
        let f = s.to_floats();
        assert_eq!(f.len(), HUD_FLOAT_COUNT);
        let back = HudSnapshot::from_floats(&f);
        assert!((back.yaw - s.yaw).abs() < 1e-4);
        assert!((back.fov_x - s.fov_x).abs() < 1e-3);
        assert_eq!(back.ids, s.ids);
        assert_eq!(back.gyro, s.gyro);
        assert_eq!(back.converging, s.converging);
        assert!((back.position_ms - s.position_ms).abs() < 2);
        assert!((back.fps - s.fps).abs() < 1e-3);
    }

    #[test]
    fn short_float_array_is_safe() {
        let s = HudSnapshot::from_floats(&[1.0, 2.0]);
        assert_eq!(s, HudSnapshot::default());
    }

    #[test]
    fn field_indices_are_unique_and_in_range() {
        let all = [
            field::YAW,
            field::PITCH,
            field::ROLL,
            field::FOV_Y,
            field::FOV_X,
            field::MODE,
            field::COVERAGE,
            field::LAYOUT,
            field::EYE,
            field::SOURCE,
            field::AT_YAW_LIMIT,
            field::AT_PITCH_LIMIT,
            field::CONVERGING,
            field::RESISTANCE,
            field::GYRO,
            field::GYRO_READY,
            field::ZOOM,
            field::POSITION_S,
            field::DURATION_S,
            field::FPS,
        ];
        let mut sorted = all.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "duplicate field index");
        assert!(all.iter().all(|i| *i < HUD_FLOAT_COUNT));
    }

    #[test]
    fn compact_line_is_single_line() {
        let s = sample();
        let c = s.compact();
        assert!(!c.contains('\n'));
        assert!(c.contains("yaw=12.34"));
        assert!(c.contains("180 SBS"));
    }
}
