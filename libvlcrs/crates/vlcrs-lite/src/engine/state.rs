//! Shared engine state.
//!
//! [`Inner`] is the single object every engine thread holds an `Arc` to.  It is
//! deliberately built from atomics plus a few small mutexes (never nested, never
//! held across a blocking call) so the four threads — demux, audio, render and
//! sensors — cannot deadlock.

use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, AtomicU8, Ordering,
};
use std::sync::{Arc, Mutex};

use vlcrs_vr::hud::{HudSnapshot, HUD_FLOAT_COUNT};
use vlcrs_vr::{Eye, MediaHints, ProjectionMode, ViewState};

use crate::api::{MediaInfoSnapshot, Options, PlayerState};
use crate::engine::clock::Clock;
use crate::engine::format::TrackFormat;
use crate::engine::queue::PacketQueue;

/// Where the media comes from.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// A path or URI understood by `AMediaExtractor` (`file://…`, `http://…`,
    /// `content://…` when a descriptor is not available, plain paths).
    Uri(String),
    /// An already open descriptor with an offset/length (`content://` via
    /// `ParcelFileDescriptor`).
    Fd { fd: i32, offset: i64, length: i64 },
}

impl Source {
    pub(crate) fn describe(&self) -> String {
        match self {
            Source::Uri(u) => u.clone(),
            Source::Fd { fd, offset, length } => format!("fd={fd}+{offset}..{length}"),
        }
    }
}

/// Counters surfaced through the HUD and the statistics API.
#[derive(Default)]
pub(crate) struct Stats {
    pub decoded_frames: AtomicU64,
    pub rendered_frames: AtomicU64,
    pub dropped_frames: AtomicU64,
    pub audio_underruns: AtomicU64,
    pub seek_count: AtomicU64,
    pub render_fps_x100: AtomicU32,
    pub hud: Mutex<[f32; HUD_FLOAT_COUNT]>,
    pub hud_full: Mutex<HudSnapshot>,
}

/// Engine state shared by all threads.
pub(crate) struct Inner {
    /// Opaque handle reported back to the application.
    pub handle: i64,
    /// JNI bridges (events + SurfaceTexture helper).
    pub bridge: Arc<crate::platform::jni::JavaBridge>,
    /// Application package name, needed by `ASensorManager`.
    pub package: String,
    /// Engine options.
    pub options: Mutex<Options>,

    // ---- lifecycle ----
    pub running: AtomicBool,
    pub play_requested: AtomicBool,
    pub paused: AtomicBool,
    pub stop_requested: AtomicBool,
    pub state: AtomicI32,
    pub seek_request_ms: AtomicI64,
    pub epoch: AtomicU64,

    // ---- media ----
    pub source: Mutex<Option<Source>>,
    pub video_format: Mutex<Option<TrackFormat>>,
    pub audio_format: Mutex<Option<TrackFormat>>,
    pub media_ready: AtomicBool,
    pub duration_us: AtomicI64,
    pub info: Mutex<MediaInfoSnapshot>,
    pub hints: Mutex<MediaHints>,
    /// Spherical metadata as probed from the container (kept so the coverage can
    /// be recomputed once the decoder reports the real picture size).
    pub spherical: Mutex<Option<vlcrs_media::SphericalMetadata>>,
    pub eos: AtomicU8,

    // ---- queues ----
    pub video_q: PacketQueue,
    pub audio_q: PacketQueue,

    // ---- clock ----
    pub clock: Clock,

    // ---- surface / viewport ----
    pub surface: Mutex<Option<jni::objects::GlobalRef>>,
    pub surface_pending: AtomicBool,
    pub viewport_w: AtomicI32,
    pub viewport_h: AtomicI32,

    // ---- VR view ----
    pub view: Mutex<ViewState>,
    pub mode: AtomicI32,
    pub eye: AtomicI32,
    pub swap_eyes: AtomicBool,
    pub gyro: AtomicBool,
    pub display_rotation: AtomicI32,
    pub tracker: Mutex<Option<crate::platform::sensors::HeadTracker>>,
    pub volume: AtomicU32,

    /// Bitmask of the boundary flags already reported, so that
    /// [`crate::api::EventType::BoundaryReached`] is only posted on transitions.
    pub boundary_posted: AtomicI32,

    pub stats: Stats,
}

impl Inner {
    pub(crate) fn new(
        handle: i64,
        bridge: Arc<crate::platform::jni::JavaBridge>,
        package: String,
        options: Options,
    ) -> Arc<Inner> {
        let mode = options.mode.as_i32();
        let eye = options.eye.as_i32();
        let gyro = options.gyro;
        let swap = options.swap_eyes;
        let rot = options.display_rotation as i32;
        let mut view = ViewState::new();
        view.fov_y = options.fov_y;
        view.gyro_enabled = gyro;
        let buffer_bytes = (options.buffer_ms.max(200) as usize) * 64 * 1024 / 1000;
        Arc::new(Inner {
            handle,
            bridge,
            package,
            options: Mutex::new(options),
            running: AtomicBool::new(true),
            play_requested: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            state: AtomicI32::new(PlayerState::Idle as i32),
            seek_request_ms: AtomicI64::new(-1),
            epoch: AtomicU64::new(0),
            source: Mutex::new(None),
            video_format: Mutex::new(None),
            audio_format: Mutex::new(None),
            media_ready: AtomicBool::new(false),
            duration_us: AtomicI64::new(0),
            info: Mutex::new(MediaInfoSnapshot::default()),
            hints: Mutex::new(MediaHints::default()),
            spherical: Mutex::new(None),
            eos: AtomicU8::new(0),
            video_q: PacketQueue::new(buffer_bytes.max(1 << 20), 512),
            audio_q: PacketQueue::new(buffer_bytes.max(1 << 20), 1024),
            clock: Clock::new(),
            surface: Mutex::new(None),
            surface_pending: AtomicBool::new(false),
            viewport_w: AtomicI32::new(0),
            viewport_h: AtomicI32::new(0),
            view: Mutex::new(view),
            mode: AtomicI32::new(mode),
            eye: AtomicI32::new(eye),
            swap_eyes: AtomicBool::new(swap),
            gyro: AtomicBool::new(gyro),
            display_rotation: AtomicI32::new(rot),
            tracker: Mutex::new(None),
            volume: AtomicU32::new(1000),
            boundary_posted: AtomicI32::new(0),
            stats: Stats::default(),
        })
    }

    // ---- helpers shared by the threads ----

    pub(crate) fn set_state(&self, s: PlayerState) {
        let prev = self.state.swap(s as i32, Ordering::AcqRel);
        if prev != s as i32 {
            crate::vlog!(
                "state {} -> {}",
                PlayerState::from_i32(prev).label(),
                s.label()
            );
        }
    }

    pub(crate) fn state(&self) -> PlayerState {
        PlayerState::from_i32(self.state.load(Ordering::Acquire))
    }

    pub(crate) fn current_epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    pub(crate) fn take_seek(&self) -> Option<i64> {
        let v = self.seek_request_ms.swap(-1, Ordering::AcqRel);
        if v < 0 {
            None
        } else {
            Some(v)
        }
    }

    pub(crate) fn request_seek(&self, ms: i64) {
        self.seek_request_ms.store(ms.max(0), Ordering::Release);
    }

    /// Begin a flush: bump the epoch, empty both queues and clear the
    /// end-of-stream markers (a seek out of EOS must be able to resume).
    pub(crate) fn begin_flush(&self) -> u64 {
        let e = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.video_q.flush();
        self.audio_q.flush();
        self.clock.invalidate();
        self.reset_eos();
        self.stats.seek_count.fetch_add(1, Ordering::Relaxed);
        e
    }

    pub(crate) fn mark_eos(&self, which: u8) {
        let prev = self.eos.fetch_or(which, Ordering::AcqRel);
        if prev | which == 0b11 && prev != 0b11 {
            self.set_state(PlayerState::EndReached);
            self.post(crate::api::EventType::EndReached, 0, 0);
        }
    }

    pub(crate) fn reset_eos(&self) {
        self.eos.store(0, Ordering::Release);
    }

    /// Post an event to the application.
    pub(crate) fn post(&self, kind: crate::api::EventType, arg1: i32, arg2: i32) {
        self.bridge.post_event(self.handle, kind as i32, arg1, arg2);
    }

    pub(crate) fn projection_mode(&self) -> ProjectionMode {
        ProjectionMode::from_i32(self.mode.load(Ordering::Relaxed))
    }

    pub(crate) fn current_eye(&self) -> Eye {
        Eye::from_i32(self.eye.load(Ordering::Relaxed))
    }

    pub(crate) fn volume(&self) -> f32 {
        self.volume.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub(crate) fn duration_ms(&self) -> i64 {
        self.duration_us.load(Ordering::Relaxed) / 1000
    }
}

/// Bit for [`Inner::mark_eos`]: the video path reached the end.
pub(crate) const EOS_VIDEO: u8 = 0b01;
/// Bit for [`Inner::mark_eos`]: the audio path reached the end.
pub(crate) const EOS_AUDIO: u8 = 0b10;
