//! Platform independent engine API.
//!
//! This module compiles on every target (it is pure data + a handle registry
//! signature) so that the JNI and C layers, and the host tests, share one
//! definition of the engine's surface.

use vlcrs_vr::{Coverage, Eye, ProjectionMode, ResolvedProjection, StereoLayout};

/// Engine states, mirroring the `libvlc_MediaPlayer` state machine.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerState {
    /// Nothing loaded.
    Idle = 0,
    /// Opening the media / probing.
    Opening = 1,
    /// Media opened, not playing yet.
    Prepared = 2,
    /// Buffering (playback stalled waiting for data).
    Buffering = 3,
    /// Playing.
    Playing = 4,
    /// Paused.
    Paused = 5,
    /// Stopped by the user.
    Stopped = 6,
    /// End of media reached.
    EndReached = 7,
    /// Fatal error.
    Error = 8,
}

impl PlayerState {
    /// Convert from the integer representation used across FFI.
    pub fn from_i32(v: i32) -> PlayerState {
        match v {
            0 => PlayerState::Idle,
            1 => PlayerState::Opening,
            2 => PlayerState::Prepared,
            3 => PlayerState::Buffering,
            4 => PlayerState::Playing,
            5 => PlayerState::Paused,
            6 => PlayerState::Stopped,
            7 => PlayerState::EndReached,
            _ => PlayerState::Error,
        }
    }

    /// Short label for logs/HUD.
    pub fn label(self) -> &'static str {
        match self {
            PlayerState::Idle => "Idle",
            PlayerState::Opening => "Opening",
            PlayerState::Prepared => "Prepared",
            PlayerState::Buffering => "Buffering",
            PlayerState::Playing => "Playing",
            PlayerState::Paused => "Paused",
            PlayerState::Stopped => "Stopped",
            PlayerState::EndReached => "EndReached",
            PlayerState::Error => "Error",
        }
    }
}

/// Events delivered to the application.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventType {
    /// The media was opened. `arg1` = duration in ms (saturated to i32),
    /// `arg2` = 1 when spherical metadata was found.
    Prepared = 1,
    /// Playback started.
    Playing = 2,
    /// Playback paused.
    Paused = 3,
    /// Playback stopped.
    Stopped = 4,
    /// End of media.
    EndReached = 5,
    /// Buffering: `arg1` = percent (0…100).
    Buffering = 6,
    /// Video track information: `arg1` = width, `arg2` = height.
    VideoSize = 7,
    /// The resolved projection changed: `arg1` = mode id, `arg2` = eye id.
    ProjectionChanged = 8,
    /// The view hit a coverage boundary: `arg1` = 1 yaw, 2 pitch.
    BoundaryReached = 9,
    /// An error occurred: `arg1` = error code.
    Error = 10,
    /// The engine logged something worth surfacing: `arg1` = level.
    Log = 11,
    /// A seek completed.
    Seeked = 12,
    /// The first frame was rendered.
    FirstFrame = 13,
}

/// Error codes reported through [`EventType::Error`].
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    /// Unknown failure.
    Unknown = 0,
    /// The media could not be opened (missing file, unsupported container).
    OpenFailed = 1,
    /// No playable track was found.
    NoTracks = 2,
    /// No video decoder for the track's MIME type.
    VideoDecoderFailed = 3,
    /// The video decoder rejected the format.
    VideoConfigureFailed = 4,
    /// No audio decoder (audio will be silent, playback continues).
    AudioDecoderFailed = 5,
    /// The audio output could not be opened (playback continues, muted).
    AudioOutputFailed = 6,
    /// EGL/GL initialisation failed.
    RenderFailed = 7,
    /// The projection advertised by the file is not supported (cubemap/mesh).
    UnsupportedProjection = 8,
    /// A decode error occurred.
    DecodeFailed = 9,
}

/// Result of the engine's control calls.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Success.
    Ok = 0,
    /// Invalid handle.
    BadHandle = -1,
    /// Invalid argument.
    BadArgument = -2,
    /// Not allowed in the current state.
    BadState = -3,
    /// Operation not supported.
    Unsupported = -4,
    /// Internal failure (details in logcat).
    Failed = -5,
}

impl Status {
    /// Convert to the integer representation used across FFI.
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

/// Static information about the loaded media.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MediaInfoSnapshot {
    /// Coded width.
    pub width: i32,
    /// Coded height.
    pub height: i32,
    /// Display rotation (clockwise degrees).
    pub rotation: i32,
    /// Duration in milliseconds.
    pub duration_ms: i64,
    /// `1` when the container advertised spherical metadata.
    pub has_spherical: i32,
    /// Detected coverage id (`Coverage` → 0 planar / 1 360 / 2 180), `-1` unknown.
    pub coverage: i32,
    /// Detected stereo layout id, `-1` unknown.
    pub layout: i32,
    /// Container label hash-free index: 0 unknown, 1 mp4, 2 mkv, 3 other.
    pub container: i32,
    /// Audio sample rate.
    pub sample_rate: i32,
    /// Audio channel count.
    pub channels: i32,
    /// Average video frame rate (0 when unknown).
    pub fps: f32,
}

/// Ids used in [`MediaInfoSnapshot`] so that the JNI layer stays allocation free.
pub fn coverage_id(c: Option<Coverage>) -> i32 {
    match c {
        None => -1,
        Some(Coverage::Planar) => 0,
        Some(Coverage::Full360) => 1,
        Some(Coverage::Half180) => 2,
    }
}

/// Id of a stereo layout (`-1` when unknown).
pub fn layout_id(l: Option<StereoLayout>) -> i32 {
    match l {
        None => -1,
        Some(StereoLayout::Mono) => 0,
        Some(StereoLayout::SideBySide) => 1,
        Some(StereoLayout::TopBottom) => 2,
    }
}

/// Ids of a resolved projection, as reported to the UI.
pub fn projection_ids(p: &ResolvedProjection) -> [i32; 5] {
    [
        p.mode.as_i32(),
        coverage_id(Some(p.coverage)),
        layout_id(Some(p.layout)),
        p.eye.as_i32(),
        match p.source {
            vlcrs_vr::ProjectionSource::Forced => 0,
            vlcrs_vr::ProjectionSource::Metadata => 1,
            vlcrs_vr::ProjectionSource::Fallback => 2,
        },
    ]
}

/// Engine options, all optional.
#[derive(Clone, Debug)]
pub struct Options {
    /// Vertical FOV used when an item is opened (degrees).
    pub fov_y: f32,
    /// `true` to start with the gyroscope enabled.
    pub gyro: bool,
    /// Preferred projection mode at start.
    pub mode: ProjectionMode,
    /// Eye rendered for stereo layouts.
    pub eye: Eye,
    /// Tessellation step of the spherical meshes, in degrees.
    pub mesh_step: i32,
    /// `true` when the top/bottom (or left/right) halves are swapped.
    pub swap_eyes: bool,
    /// Target video buffer count (latency vs smoothness).
    pub frame_queue_depth: usize,
    /// Maximum buffered media ahead of the clock, in milliseconds.
    pub buffer_ms: i64,
    /// Display rotation in degrees (0/90/180/270), used by the head tracker.
    pub display_rotation: f32,
    /// Force software decoding (`c2.android.*` codecs).
    pub force_software_decoder: bool,
    /// `true` to keep decoding video while no surface is attached.
    pub decode_without_surface: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            fov_y: vlcrs_vr::view::FOV_DEFAULT_DEG,
            gyro: false,
            mode: ProjectionMode::Auto,
            eye: Eye::Left,
            mesh_step: vlcrs_vr::mesh::DEFAULT_STEP_DEG,
            swap_eyes: false,
            frame_queue_depth: 3,
            buffer_ms: 1500,
            display_rotation: 90.0,
            force_software_decoder: false,
            decode_without_surface: true,
        }
    }
}

/// Int array layout used by the JNI/C layers to publish [`MediaInfoSnapshot`]:
/// `[width, height, rotation, hasSpherical, coverage, layout, container,
/// sampleRate, channels, fps×100]`.
pub const MEDIA_INFO_FIELDS: usize = 10;

/// Flatten a [`MediaInfoSnapshot`] into the array layout above.
pub fn media_info_array(info: &MediaInfoSnapshot) -> [i32; MEDIA_INFO_FIELDS] {
    [
        info.width,
        info.height,
        info.rotation,
        info.has_spherical,
        info.coverage,
        info.layout,
        info.container,
        info.sample_rate,
        info.channels,
        (info.fps * 100.0) as i32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use vlcrs_vr::{MediaHints, Pose, ProjectionSource};

    #[test]
    fn state_ids_are_stable() {
        for (id, expected) in [
            (0, PlayerState::Idle),
            (1, PlayerState::Opening),
            (2, PlayerState::Prepared),
            (3, PlayerState::Buffering),
            (4, PlayerState::Playing),
            (5, PlayerState::Paused),
            (6, PlayerState::Stopped),
            (7, PlayerState::EndReached),
            (8, PlayerState::Error),
            (99, PlayerState::Error),
        ] {
            assert_eq!(PlayerState::from_i32(id), expected);
        }
        assert_eq!(PlayerState::Playing as i32, 4);
        assert_eq!(PlayerState::Playing.label(), "Playing");
    }

    #[test]
    fn status_and_error_codes_fit_in_i32() {
        assert_eq!(Status::Ok.as_i32(), 0);
        assert!(Status::Failed.as_i32() < 0);
        assert_eq!(ErrorCode::UnsupportedProjection as i32, 8);
        assert_eq!(EventType::FirstFrame as i32, 13);
    }

    #[test]
    fn options_defaults_are_sane() {
        let o = Options::default();
        assert!((o.fov_y - vlcrs_vr::view::FOV_DEFAULT_DEG).abs() < 1e-3);
        assert_eq!(o.mode, ProjectionMode::Auto);
        assert_eq!(o.eye, Eye::Left);
        assert!(o.frame_queue_depth >= 2);
        assert!(o.buffer_ms >= 500);
        assert_eq!(o.display_rotation, 90.0);
    }

    #[test]
    fn id_helpers() {
        assert_eq!(coverage_id(None), -1);
        assert_eq!(coverage_id(Some(Coverage::Planar)), 0);
        assert_eq!(coverage_id(Some(Coverage::Full360)), 1);
        assert_eq!(coverage_id(Some(Coverage::Half180)), 2);
        assert_eq!(layout_id(None), -1);
        assert_eq!(layout_id(Some(StereoLayout::TopBottom)), 2);
    }

    #[test]
    fn projection_ids_round_trip() {
        let hints = MediaHints {
            width: 3840,
            height: 1920,
            rotation: 0,
            stereo: Some(StereoLayout::SideBySide),
            swap_eyes: false,
            coverage: Some(Coverage::Half180),
            pose: Some(Pose::default()),
        };
        let p = ResolvedProjection::resolve(ProjectionMode::Auto, &hints, Eye::Right);
        let ids = projection_ids(&p);
        assert_eq!(ids[0], ProjectionMode::Auto.as_i32());
        assert_eq!(ids[1], 2, "180 coverage");
        assert_eq!(ids[2], 1, "SBS layout");
        assert_eq!(ids[3], Eye::Right.as_i32());
        assert_eq!(ids[4], 1, "metadata");
        assert_eq!(p.source, ProjectionSource::Metadata, "{}", p.describe());
    }

    #[test]
    fn media_info_array_layout_is_stable() {
        let info = MediaInfoSnapshot {
            width: 3840,
            height: 1920,
            rotation: 90,
            duration_ms: 1000,
            has_spherical: 1,
            coverage: 2,
            layout: 1,
            container: 1,
            sample_rate: 48_000,
            channels: 6,
            fps: 59.94,
        };
        assert_eq!(
            media_info_array(&info),
            [3840, 1920, 90, 1, 2, 1, 1, 48_000, 6, 5994]
        );
        assert_eq!(media_info_array(&info).len(), MEDIA_INFO_FIELDS);
    }

    #[test]
    fn media_info_snapshot_defaults_are_unknown() {
        let m = MediaInfoSnapshot::default();
        assert_eq!(m.width, 0);
        assert_eq!(m.has_spherical, 0);
        assert_eq!(m.duration_ms, 0);
    }
}
