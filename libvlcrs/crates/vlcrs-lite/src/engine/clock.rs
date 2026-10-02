//! Master clock.
//!
//! Audio is the master when a track exists (like every serious player, and like
//! the reference implementation): the audio thread publishes the media time of
//! the last sample handed to AAudio, and everybody else interpolates between
//! two publications with a monotonic timer.  Without audio the video path
//! publishes the clock itself.

// The engine is Android-only; on host builds these helpers exist for the unit
// tests below.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

fn mono_ns() -> i64 {
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_nanos() as i64
}

/// Monotonic nanoseconds since an arbitrary epoch (used by the tests).
#[allow(dead_code)]
pub(crate) fn monotonic_ns() -> i64 {
    mono_ns()
}

pub(crate) struct Clock {
    pts_us: AtomicI64,
    system_ns: AtomicI64,
    paused: AtomicBool,
    audio_master: AtomicBool,
    valid: AtomicBool,
}

impl Clock {
    pub(crate) fn new() -> Clock {
        Clock {
            pts_us: AtomicI64::new(0),
            system_ns: AtomicI64::new(mono_ns()),
            paused: AtomicBool::new(false),
            audio_master: AtomicBool::new(false),
            valid: AtomicBool::new(false),
        }
    }

    /// Audio drives the clock.
    pub(crate) fn set_audio_master(&self, v: bool) {
        self.audio_master.store(v, Ordering::Relaxed);
    }

    pub(crate) fn is_audio_master(&self) -> bool {
        self.audio_master.load(Ordering::Relaxed)
    }

    /// Publish the audio position (media time of the sample being rendered now).
    pub(crate) fn set_audio(&self, pts_us: i64) {
        self.pts_us.store(pts_us, Ordering::Relaxed);
        self.system_ns.store(mono_ns(), Ordering::Relaxed);
        self.valid.store(true, Ordering::Relaxed);
    }

    /// Publish the video position (only honoured when there is no audio).
    pub(crate) fn set_video(&self, pts_us: i64) {
        if self.audio_master.load(Ordering::Relaxed) {
            return;
        }
        self.pts_us.store(pts_us, Ordering::Relaxed);
        self.system_ns.store(mono_ns(), Ordering::Relaxed);
        self.valid.store(true, Ordering::Relaxed);
    }

    /// Freeze/resume interpolation.
    pub(crate) fn set_paused(&self, paused: bool) {
        if paused == self.paused.load(Ordering::Relaxed) {
            return;
        }
        if paused {
            // latch the current interpolated value so it does not drift
            let now = self.now_us();
            self.pts_us.store(now, Ordering::Relaxed);
        }
        self.paused.store(paused, Ordering::Relaxed);
        self.system_ns.store(mono_ns(), Ordering::Relaxed);
    }

    /// Current media time in microseconds.
    pub(crate) fn now_us(&self) -> i64 {
        let pts = self.pts_us.load(Ordering::Relaxed);
        if !self.valid.load(Ordering::Relaxed) {
            return 0;
        }
        if self.paused.load(Ordering::Relaxed) {
            return pts;
        }
        let elapsed_ns = mono_ns().saturating_sub(self.system_ns.load(Ordering::Relaxed));
        pts + elapsed_ns / 1000
    }

    /// Reset after a seek/flush: the next published value re-anchors the clock.
    pub(crate) fn invalidate(&self) {
        self.valid.store(false, Ordering::Relaxed);
        self.pts_us.store(0, Ordering::Relaxed);
    }

    /// Force the clock to a position (used right after a seek so that the HUD
    /// and the video pacing agree before the first sample is rendered).
    pub(crate) fn force(&self, pts_us: i64) {
        self.pts_us.store(pts_us, Ordering::Relaxed);
        self.system_ns.store(mono_ns(), Ordering::Relaxed);
        self.valid.store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.valid.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn clock_interpolates_between_updates() {
        let c = Clock::new();
        assert_eq!(c.now_us(), 0);
        c.set_audio(1_000_000);
        assert!(c.is_valid());
        sleep(Duration::from_millis(30));
        let now = c.now_us();
        assert!(
            (1_020_000..1_200_000).contains(&now),
            "clock did not advance: {now}"
        );
    }

    #[test]
    fn paused_clock_is_frozen() {
        let c = Clock::new();
        c.set_audio(500_000);
        c.set_paused(true);
        let a = c.now_us();
        sleep(Duration::from_millis(20));
        let b = c.now_us();
        assert_eq!(a, b);
        c.set_paused(false);
        sleep(Duration::from_millis(20));
        assert!(c.now_us() > b);
    }

    #[test]
    fn video_is_ignored_while_audio_is_master() {
        let c = Clock::new();
        c.set_audio_master(true);
        c.set_audio(1_000_000);
        c.set_video(9_000_000);
        assert!(c.now_us() < 2_000_000);
        c.set_audio_master(false);
        c.set_video(9_000_000);
        assert!(c.now_us() >= 9_000_000);
    }

    #[test]
    fn force_and_invalidate() {
        let c = Clock::new();
        c.force(2_000_000);
        assert_eq!(c.now_us(), 2_000_000);
        c.invalidate();
        assert!(!c.is_valid());
        assert_eq!(c.now_us(), 0);
    }

    #[test]
    fn monotonic_is_stable() {
        let a = monotonic_ns();
        sleep(Duration::from_millis(5));
        let b = monotonic_ns();
        assert!(b > a);
    }
}
