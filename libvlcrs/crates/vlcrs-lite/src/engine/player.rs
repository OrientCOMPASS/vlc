//! The engine's control surface: player lifecycle, transport, VR controls and
//! the handle registry used by the JNI / C layers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use jni::objects::GlobalRef;
use vlcrs_vr::hud::HudSnapshot;
use vlcrs_vr::projection::ResolvedProjection;
use vlcrs_vr::view::Viewport;
use vlcrs_vr::{Eye, MediaHints, ProjectionMode};

use crate::api::{MediaInfoSnapshot, Options, PlayerState, Status};
use crate::engine::audio::audio_thread;
use crate::engine::demux::read_thread;
use crate::engine::state::Inner;
use crate::engine::Source;
use crate::platform::jni::JavaBridge;
use crate::platform::sensors::HeadTracker;
use crate::render::renderer::render_thread;

static REGISTRY: OnceLock<Mutex<HashMap<i64, Arc<Player>>>> = OnceLock::new();
static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

fn registry() -> &'static Mutex<HashMap<i64, Arc<Player>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A playback engine instance.
pub struct Player {
    inner: Arc<Inner>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

impl Player {
    /// Create an engine, start its threads and register it.
    ///
    /// Returns the opaque handle used by every other call.
    pub fn create(bridge: Arc<JavaBridge>, package: String, options: Options) -> i64 {
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        let inner = Inner::new(handle, bridge, package, options);
        let player = Arc::new(Player {
            inner: Arc::clone(&inner),
            threads: Mutex::new(Vec::new()),
        });
        let mut threads = Vec::with_capacity(3);
        for (name, f) in [
            ("vlcrs-read", read_thread as fn(Arc<Inner>)),
            ("vlcrs-audio", audio_thread as fn(Arc<Inner>)),
            ("vlcrs-render", render_thread as fn(Arc<Inner>)),
        ] {
            match std::thread::Builder::new().name(name.to_string()).spawn({
                let inner = Arc::clone(&inner);
                move || f(inner)
            }) {
                Ok(h) => threads.push(h),
                Err(e) => crate::verror!("cannot spawn {name}: {e}"),
            }
        }
        if let Ok(mut t) = player.threads.lock() {
            *t = threads;
        }
        // start the head tracker when requested by the options
        if inner.gyro.load(Ordering::Relaxed) {
            player.start_tracker();
        }
        if let Ok(mut reg) = registry().lock() {
            reg.insert(handle, player);
        }
        crate::vlog!("player {handle} created (libvlcrs {})", crate::VERSION);
        handle
    }

    /// Look up a player by handle.
    pub fn get(handle: i64) -> Option<Arc<Player>> {
        registry().lock().ok().and_then(|g| g.get(&handle).cloned())
    }

    /// Destroy a player, stopping and joining its threads.
    pub fn destroy(handle: i64) -> Status {
        let player = match registry().lock() {
            Ok(mut g) => g.remove(&handle),
            Err(_) => None,
        };
        let Some(player) = player else {
            return Status::BadHandle;
        };
        player.shutdown();
        crate::vlog!("player {handle} destroyed");
        Status::Ok
    }

    fn shutdown(&self) {
        self.inner.running.store(false, Ordering::Release);
        self.inner.play_requested.store(false, Ordering::Release);
        self.inner.stop_requested.store(true, Ordering::Release);
        self.inner.video_q.abort();
        self.inner.audio_q.abort();
        if let Ok(mut g) = self.inner.tracker.lock() {
            if let Some(mut t) = g.take() {
                t.stop();
            }
        }
        // release the application surface reference
        if let Ok(mut g) = self.inner.surface.lock() {
            *g = None;
        }
        self.inner.surface_pending.store(true, Ordering::Release);
        if let Ok(mut threads) = self.threads.lock() {
            for t in threads.drain(..) {
                let _ = t.join();
            }
        }
    }

    // ----------------------------------------------------------------- media

    /// Set a URI or path (`file://…`, `http://…`, plain path).
    pub fn set_media_uri(&self, uri: &str) -> Status {
        if uri.is_empty() {
            return Status::BadArgument;
        }
        self.reset_for_new_media();
        if let Ok(mut g) = self.inner.source.lock() {
            *g = Some(Source::Uri(uri.to_string()));
        }
        Status::Ok
    }

    /// Set an open descriptor with an offset/length (`content://` URIs).
    pub fn set_media_fd(&self, fd: i32, offset: i64, length: i64) -> Status {
        if fd < 0 {
            return Status::BadArgument;
        }
        self.reset_for_new_media();
        if let Ok(mut g) = self.inner.source.lock() {
            *g = Some(Source::Fd { fd, offset, length });
        }
        Status::Ok
    }

    fn reset_for_new_media(&self) {
        self.inner.media_ready.store(false, Ordering::Release);
        self.inner.reset_eos();
        self.inner.clock.invalidate();
        self.inner.video_q.reset();
        self.inner.audio_q.reset();
        self.inner.video_q.flush();
        self.inner.audio_q.flush();
        self.inner.duration_us.store(0, Ordering::Release);
        if let Ok(mut g) = self.inner.video_format.lock() {
            *g = None;
        }
        if let Ok(mut g) = self.inner.audio_format.lock() {
            *g = None;
        }
        if let Ok(mut g) = self.inner.hints.lock() {
            *g = MediaHints::default();
        }
        if let Ok(mut g) = self.inner.spherical.lock() {
            *g = None;
        }
        if let Ok(mut g) = self.inner.view.lock() {
            g.raw_yaw = 0.0;
            g.raw_pitch = 0.0;
            g.roll = 0.0;
            g.set_home(Default::default());
        }
        if let Ok(g) = self.inner.tracker.lock() {
            if let Some(t) = g.as_ref() {
                t.reset();
            }
        }
        self.inner.set_state(PlayerState::Idle);
    }

    /// Start (or restart) playback of the current media.
    pub fn play(&self) -> Status {
        let has_source = self
            .inner
            .source
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false);
        if !has_source {
            return Status::BadState;
        }
        self.inner.stop_requested.store(false, Ordering::Release);
        self.inner.paused.store(false, Ordering::Release);
        self.inner.clock.set_paused(false);
        self.inner.reset_eos();
        self.inner.play_requested.store(true, Ordering::Release);
        Status::Ok
    }

    /// Pause playback (the view can still be explored).
    pub fn pause(&self) -> Status {
        if !self.inner.media_ready.load(Ordering::Acquire) {
            return Status::BadState;
        }
        self.inner.paused.store(true, Ordering::Release);
        self.inner.clock.set_paused(true);
        self.inner.set_state(PlayerState::Paused);
        self.inner.post(crate::api::EventType::Paused, 0, 0);
        Status::Ok
    }

    /// Resume playback.
    pub fn resume(&self) -> Status {
        if !self.inner.media_ready.load(Ordering::Acquire) {
            return Status::BadState;
        }
        self.inner.paused.store(false, Ordering::Release);
        self.inner.clock.set_paused(false);
        self.inner.set_state(PlayerState::Playing);
        self.inner.post(crate::api::EventType::Playing, 0, 0);
        Status::Ok
    }

    /// Stop playback and unload the media.
    pub fn stop(&self) -> Status {
        self.inner.play_requested.store(false, Ordering::Release);
        self.inner.stop_requested.store(true, Ordering::Release);
        self.inner.paused.store(false, Ordering::Release);
        self.inner.video_q.flush();
        self.inner.audio_q.flush();
        self.inner.set_state(PlayerState::Stopped);
        self.inner.media_ready.store(false, Ordering::Release);
        self.inner.reset_eos();
        self.inner.clock.invalidate();
        self.inner.post(crate::api::EventType::Stopped, 0, 0);
        Status::Ok
    }

    /// Seek to an absolute position in milliseconds.
    pub fn seek_to(&self, ms: i64) -> Status {
        if !self.inner.media_ready.load(Ordering::Acquire) {
            return Status::BadState;
        }
        let duration = self.inner.duration_ms();
        let target = if duration > 0 {
            ms.clamp(0, duration)
        } else {
            ms.max(0)
        };
        self.inner.request_seek(target);
        Status::Ok
    }

    /// Current position in milliseconds.
    pub fn time_ms(&self) -> i64 {
        let pending = self.inner.seek_request_ms.load(Ordering::Acquire);
        if pending >= 0 {
            return pending;
        }
        (self.inner.clock.now_us() / 1000).max(0)
    }

    /// Duration in milliseconds (0 when unknown).
    pub fn length_ms(&self) -> i64 {
        self.inner.duration_ms()
    }

    /// Current state.
    pub fn state(&self) -> PlayerState {
        self.inner.state()
    }

    /// Set the volume, `0.0 … 1.0`.
    pub fn set_volume(&self, volume: f32) -> Status {
        let v = volume.clamp(0.0, 1.0);
        self.inner
            .volume
            .store((v * 1000.0) as u32, Ordering::Release);
        Status::Ok
    }

    // --------------------------------------------------------------- surface

    /// Attach (or detach, with `None`) the application's output surface.
    pub fn set_surface(&self, surface: Option<GlobalRef>) -> Status {
        if let Ok(mut g) = self.inner.surface.lock() {
            *g = surface;
        }
        self.inner.surface_pending.store(true, Ordering::Release);
        Status::Ok
    }

    /// Notify a surface size change (the EGL surface is re-queried).
    pub fn surface_changed(&self, width: i32, height: i32) -> Status {
        self.inner.viewport_w.store(width, Ordering::Release);
        self.inner.viewport_h.store(height, Ordering::Release);
        self.inner.surface_pending.store(true, Ordering::Release);
        Status::Ok
    }

    // -------------------------------------------------------------------- VR

    /// Select a projection mode; takes effect on the next frame, in place.
    pub fn set_projection_mode(&self, mode: i32) -> Status {
        let m = ProjectionMode::from_i32(mode);
        self.inner.mode.store(m.as_i32(), Ordering::Release);
        Status::Ok
    }

    /// The active projection mode.
    pub fn projection_mode(&self) -> ProjectionMode {
        self.inner.projection_mode()
    }

    /// Select the eye rendered from a stereo layout.
    pub fn set_eye(&self, eye: i32) -> Status {
        self.inner
            .eye
            .store(Eye::from_i32(eye).as_i32(), Ordering::Release);
        Status::Ok
    }

    /// The active eye.
    pub fn eye(&self) -> Eye {
        self.inner.current_eye()
    }

    /// Invert the eye packing order advertised by the container.
    pub fn set_swap_eyes(&self, swap: bool) -> Status {
        self.inner.swap_eyes.store(swap, Ordering::Release);
        Status::Ok
    }

    /// Set the vertical field of view in degrees.
    pub fn set_fov(&self, fov_y: f32) -> Status {
        if !fov_y.is_finite() || fov_y <= 0.0 {
            return Status::BadArgument;
        }
        if let Ok(mut v) = self.inner.view.lock() {
            v.set_fov(fov_y);
        }
        Status::Ok
    }

    /// Pinch zoom: `factor > 1` zooms in.
    pub fn zoom(&self, factor: f32) -> Status {
        if !factor.is_finite() || factor <= 0.0 {
            return Status::BadArgument;
        }
        if let Ok(mut v) = self.inner.view.lock() {
            v.zoom(factor);
        }
        Status::Ok
    }

    /// Single finger drag, in pixels.
    pub fn drag_pixels(&self, dx: f32, dy: f32) -> Status {
        let vp = self.viewport();
        if let Ok(mut v) = self.inner.view.lock() {
            let fov = v.fov_y;
            v.drag_pixels(dx, dy, vp, fov);
        }
        Status::Ok
    }

    /// Single finger drag, in degrees.
    pub fn drag_degrees(&self, dyaw: f32, dpitch: f32) -> Status {
        if let Ok(mut v) = self.inner.view.lock() {
            v.drag_degrees(dyaw, dpitch);
        }
        Status::Ok
    }

    /// Enable/disable gyroscope driven look-around.
    pub fn set_gyro(&self, enabled: bool) -> Status {
        self.inner.gyro.store(enabled, Ordering::Release);
        if let Ok(mut v) = self.inner.view.lock() {
            v.gyro_enabled = enabled;
        }
        if enabled {
            self.start_tracker();
        } else {
            self.stop_tracker();
        }
        Status::Ok
    }

    fn start_tracker(&self) {
        let mut guard = match self.inner.tracker.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if guard.is_some() {
            return;
        }
        *guard = HeadTracker::start(&self.inner.package);
        if guard.is_none() {
            crate::vwarn!("head tracker unavailable on this device");
            return;
        }
        let rot = self.inner.display_rotation.load(Ordering::Relaxed) as f32;
        if let Some(t) = guard.as_mut() {
            t.set_display_rotation(rot);
        }
    }

    fn stop_tracker(&self) {
        if let Ok(mut g) = self.inner.tracker.lock() {
            if let Some(mut t) = g.take() {
                t.stop();
            }
        }
    }

    /// "视角摆正": recentre the view (manual offsets and head tracker).
    pub fn recenter(&self) -> Status {
        if let Ok(mut v) = self.inner.view.lock() {
            v.recenter();
        }
        if let Ok(g) = self.inner.tracker.lock() {
            if let Some(t) = g.as_ref() {
                t.reset();
            }
        }
        Status::Ok
    }

    /// Set the display rotation (0/90/180/270) used by the head tracker.
    pub fn set_display_rotation(&self, deg: i32) -> Status {
        self.inner
            .display_rotation
            .store(deg.rem_euclid(360), Ordering::Release);
        if let Ok(mut g) = self.inner.tracker.lock() {
            if let Some(t) = g.as_mut() {
                t.set_display_rotation(deg as f32);
            }
        }
        Status::Ok
    }

    /// Current viewport as seen by the renderer.
    pub fn viewport(&self) -> Viewport {
        Viewport {
            width: self.inner.viewport_w.load(Ordering::Relaxed).max(0) as u32,
            height: self.inner.viewport_h.load(Ordering::Relaxed).max(0) as u32,
        }
    }

    /// Fill `out` with the compact HUD representation (see
    /// [`vlcrs_vr::hud::field`]) and return how many floats were written.
    pub fn view_info(&self, out: &mut [f32]) -> usize {
        let Ok(g) = self.inner.stats.hud.lock() else {
            return 0;
        };
        let n = out.len().min(g.len());
        out[..n].copy_from_slice(&g[..n]);
        n
    }

    /// The full HUD snapshot.
    pub fn hud(&self) -> HudSnapshot {
        self.inner
            .stats
            .hud_full
            .lock()
            .map(|g| *g)
            .unwrap_or_default()
    }

    /// Static information about the loaded media.
    pub fn media_info(&self) -> MediaInfoSnapshot {
        self.inner.info.lock().map(|g| *g).unwrap_or_default()
    }

    /// The projection that the renderer currently resolves to.
    pub fn resolved_projection(&self) -> ResolvedProjection {
        let hints = self.inner.hints.lock().map(|g| *g).unwrap_or_default();
        let mut r = ResolvedProjection::resolve(
            self.inner.projection_mode(),
            &hints,
            self.inner.current_eye(),
        );
        if self.inner.swap_eyes.load(Ordering::Relaxed) != hints.swap_eyes {
            r.uv = vlcrs_vr::UvRect::for_layout(r.layout, r.eye, true);
        }
        r
    }

    /// Runtime counters (diagnostics / HUD).
    pub fn stats(&self) -> PlayerStats {
        PlayerStats {
            decoded_frames: self.inner.stats.decoded_frames.load(Ordering::Relaxed),
            rendered_frames: self.inner.stats.rendered_frames.load(Ordering::Relaxed),
            dropped_frames: self.inner.stats.dropped_frames.load(Ordering::Relaxed),
            audio_underruns: self.inner.stats.audio_underruns.load(Ordering::Relaxed),
            seeks: self.inner.stats.seek_count.load(Ordering::Relaxed),
            fps: self.inner.stats.render_fps_x100.load(Ordering::Relaxed) as f32 / 100.0,
            buffered_ms: self
                .inner
                .video_q
                .buffered_ms()
                .max(self.inner.audio_q.buffered_ms()),
            events_posted: self.inner.bridge.posted(),
        }
    }

    /// Access to the shared engine state (used by the FFI layers).
    pub fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Counters exposed through the statistics API.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerStats {
    /// Frames decoded.
    pub decoded_frames: u64,
    /// Frames rendered.
    pub rendered_frames: u64,
    /// Frames dropped for being late.
    pub dropped_frames: u64,
    /// Audio buffers that could not be written.
    pub audio_underruns: u64,
    /// Number of seeks performed.
    pub seeks: u64,
    /// Rendered frames per second.
    pub fps: f32,
    /// Buffered media ahead of the clock, in milliseconds.
    pub buffered_ms: i64,
    /// Events delivered to the application.
    pub events_posted: i64,
}
