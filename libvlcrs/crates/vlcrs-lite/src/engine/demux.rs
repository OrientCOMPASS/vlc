//! Demux thread: `AMediaExtractor` → per-track packet queues.
//!
//! The thread also owns the *container probing* pass ([`vlcrs_media`]) that
//! extracts the spherical metadata driving the `Auto` projection mode, because
//! that is the only place where a seekable view of the file is available.

use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::api::{ErrorCode, EventType, MediaInfoSnapshot, PlayerState};
use crate::engine::format::TrackFormat;
use crate::engine::queue::Packet;
use crate::engine::{Inner, Source, EOS_AUDIO, EOS_VIDEO};
use crate::platform::media::{set_thread_name, Extractor};
use crate::platform::ndk::{seek_mode, AMEDIA_OK};

/// Poll interval while idle (µs).
const IDLE: Duration = Duration::from_millis(20);
/// Timeout used when a queue is full: long enough to ride out a stall, short
/// enough to notice stop/seek requests.
const PUSH_TIMEOUT: Duration = Duration::from_millis(250);
/// Upper bound for one compressed packet.
const MAX_PACKET: usize = 16 * 1024 * 1024;

pub(crate) fn read_thread(inner: Arc<Inner>) {
    set_thread_name("vlcrs-read");
    while inner.running.load(Ordering::Acquire) {
        let want = inner.play_requested.load(Ordering::Acquire)
            && !inner.stop_requested.load(Ordering::Acquire);
        if !want {
            std::thread::sleep(IDLE);
            continue;
        }
        inner.stop_requested.store(false, Ordering::Release);
        match run_session(&inner) {
            Ok(end) => {
                if end == SessionEnd::Stopped {
                    finish_stop(&inner);
                }
            }
            Err(e) => {
                crate::verror!("demux session failed: {e}");
                inner.set_state(PlayerState::Error);
                inner.post(
                    EventType::Error,
                    match e.contains("setDataSource") {
                        true => ErrorCode::OpenFailed as i32,
                        false => ErrorCode::Unknown as i32,
                    },
                    0,
                );
                finish_stop(&inner);
            }
        }
        inner.play_requested.store(false, Ordering::Release);
        std::thread::sleep(IDLE);
    }
    crate::vlog!("read thread exit");
}

#[derive(PartialEq, Eq, Debug)]
enum SessionEnd {
    Eof,
    Stopped,
}

fn finish_stop(inner: &Arc<Inner>) {
    inner.media_ready.store(false, Ordering::Release);
    inner.reset_eos();
    inner.clock.invalidate();
    inner.video_q.flush();
    inner.audio_q.flush();
    if let Ok(mut g) = inner.video_format.lock() {
        *g = None;
    }
    if let Ok(mut g) = inner.audio_format.lock() {
        *g = None;
    }
    inner.set_state(PlayerState::Stopped);
}

fn run_session(inner: &Arc<Inner>) -> Result<SessionEnd, String> {
    let source = inner
        .source
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .ok_or_else(|| "no media set".to_string())?;
    crate::vlog!("opening {}", source.describe());
    inner.set_state(PlayerState::Opening);
    inner.reset_eos();
    inner.video_q.reset();
    inner.audio_q.reset();
    let epoch = inner.begin_flush();

    let mut ex = Extractor::new().ok_or("AMediaExtractor_new failed")?;
    let st = match &source {
        Source::Uri(uri) => ex.set_data_source(uri),
        Source::Fd { fd, offset, length } => ex.set_data_source_fd(*fd, *offset, *length),
    };
    if st != AMEDIA_OK {
        return Err(format!(
            "setDataSource failed: {st} ({})",
            crate::platform::ndk::media_status_name(st)
        ));
    }

    // ---- container probing (spherical metadata, duration) ----
    let probed = probe_container(&source);

    // ---- track selection ----
    let count = ex.track_count();
    let mut video_idx: Option<usize> = None;
    let mut audio_idx: Option<usize> = None;
    let mut video_tf: Option<TrackFormat> = None;
    let mut audio_tf: Option<TrackFormat> = None;
    for i in 0..count.min(32) {
        let Some(fmt) = ex.track_format(i) else {
            continue;
        };
        let tf = TrackFormat::from(&fmt);
        crate::vlog!("track {i}: {}", tf.describe());
        if tf.is_video() && video_idx.is_none() {
            video_idx = Some(i);
            video_tf = Some(tf);
        } else if tf.is_audio() && audio_idx.is_none() {
            audio_idx = Some(i);
            audio_tf = Some(tf);
        }
    }
    if video_idx.is_none() && audio_idx.is_none() {
        return Err("no playable track".to_string());
    }
    if let Some(i) = video_idx {
        let st = ex.select_track(i);
        if st != AMEDIA_OK {
            crate::vwarn!("selectTrack(video {i}) failed: {st}");
        }
    }
    if let Some(i) = audio_idx {
        let st = ex.select_track(i);
        if st != AMEDIA_OK {
            crate::vwarn!("selectTrack(audio {i}) failed: {st}");
        }
    }

    // ---- publish ----
    let duration_us: i64 = probed
        .as_ref()
        .and_then(|p| p.duration_ms)
        .map(|ms| ms as i64 * 1000)
        .filter(|d| *d > 0)
        .or_else(|| video_tf.as_ref().map(|t| t.duration_us).filter(|d| *d > 0))
        .or_else(|| audio_tf.as_ref().map(|t| t.duration_us).filter(|d| *d > 0))
        .unwrap_or(0);
    inner.duration_us.store(duration_us, Ordering::Release);

    let mut hints = probed
        .as_ref()
        .map(|p| p.hints())
        .unwrap_or_else(vlcrs_vr::MediaHints::default);
    let spherical = probed
        .as_ref()
        .and_then(|p| p.video().and_then(|t| t.spherical));
    // the platform knows the coded size/rotation better than our prober
    if let Some(t) = &video_tf {
        if t.width > 0 {
            hints.width = t.width as u32;
        }
        if t.height > 0 {
            hints.height = t.height as u32;
        }
        if t.rotation != 0 {
            hints.rotation = t.rotation;
        }
        if let Some(s) = spherical {
            hints.coverage = s.coverage(hints.width, hints.height);
        }
    }
    if let Ok(mut g) = inner.hints.lock() {
        *g = hints;
    }
    if let Ok(mut g) = inner.spherical.lock() {
        *g = spherical;
    }
    if let Ok(mut g) = inner.video_format.lock() {
        *g = video_tf.clone();
    }
    if let Ok(mut g) = inner.audio_format.lock() {
        *g = audio_tf.clone();
    }

    let mut snapshot = MediaInfoSnapshot {
        width: video_tf.as_ref().map(|t| t.width).unwrap_or(0),
        height: video_tf.as_ref().map(|t| t.height).unwrap_or(0),
        rotation: hints.rotation,
        duration_ms: duration_us / 1000,
        has_spherical: i32::from(spherical.is_some()),
        coverage: crate::api::coverage_id(hints.coverage),
        layout: crate::api::layout_id(hints.stereo),
        container: probed
            .as_ref()
            .map(|p| match p.container {
                vlcrs_media::Container::Mp4 => 1,
                vlcrs_media::Container::Matroska => 2,
                vlcrs_media::Container::Unknown => 0,
            })
            .unwrap_or(3),
        sample_rate: audio_tf.as_ref().map(|t| t.sample_rate).unwrap_or(0),
        channels: audio_tf.as_ref().map(|t| t.channels).unwrap_or(0),
        fps: video_tf.as_ref().map(|t| t.frame_rate).unwrap_or(0.0),
    };
    if let Some(p) = &probed {
        snapshot.duration_ms = p
            .duration_ms
            .map(|d| d as i64)
            .unwrap_or(snapshot.duration_ms);
    }
    if let Ok(mut g) = inner.info.lock() {
        *g = snapshot;
    }

    inner.clock.set_audio_master(audio_idx.is_some());
    inner.clock.invalidate();
    if video_idx.is_none() {
        inner.mark_eos(EOS_VIDEO);
    }
    if audio_idx.is_none() {
        inner.mark_eos(EOS_AUDIO);
    }
    inner.media_ready.store(true, Ordering::Release);
    inner.set_state(PlayerState::Playing);
    inner.post(
        EventType::Prepared,
        (duration_us / 1000).min(i64::from(i32::MAX)) as i32,
        i32::from(spherical.is_some()),
    );
    if let Some(t) = &video_tf {
        if t.width > 0 && t.height > 0 {
            inner.post(EventType::VideoSize, t.width, t.height);
        }
    }
    crate::vlog!(
        "prepared: {} ({} ms), spherical={:?}",
        probed
            .as_ref()
            .map(|p| p.summary())
            .unwrap_or_else(|| "unprobed".into()),
        duration_us / 1000,
        spherical.map(|s| s.projection),
    );

    read_loop(inner, &mut ex, video_idx, audio_idx, epoch)
}

fn read_loop(
    inner: &Arc<Inner>,
    ex: &mut Extractor,
    video_idx: Option<usize>,
    audio_idx: Option<usize>,
    mut epoch: u64,
) -> Result<SessionEnd, String> {
    let max_packet = {
        let v = inner
            .video_format
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .map(|t| t.max_input_size.max(0) as usize)
            .unwrap_or(0);
        v.clamp(256 * 1024, MAX_PACKET)
    };
    let mut buf = vec![0u8; max_packet];
    let mut samples: u64 = 0;
    let mut last_pts = 0i64;

    loop {
        if !inner.running.load(Ordering::Acquire) {
            return Ok(SessionEnd::Stopped);
        }
        if inner.stop_requested.load(Ordering::Acquire) {
            crate::vlog!("demux: stop requested after {samples} samples");
            return Ok(SessionEnd::Stopped);
        }
        if let Some(ms) = inner.take_seek() {
            epoch = inner.begin_flush();
            let target_us = ms.saturating_mul(1000);
            let st = ex.seek_to(target_us, seek_mode::CLOSEST_SYNC);
            if st != AMEDIA_OK {
                crate::vwarn!("seekTo({ms}ms) failed: {st}");
            }
            inner.clock.force(target_us);
            inner.post(EventType::Seeked, (ms.min(i64::from(i32::MAX))) as i32, 0);
            crate::vlog!("seek to {ms} ms (epoch {epoch})");
            continue;
        }

        let track = ex.sample_track_index();
        if track < 0 {
            // end of stream: hand EOS markers to both consumers
            if video_idx.is_some() {
                let eos = Packet {
                    data: Vec::new(),
                    pts_us: last_pts,
                    flags: 0,
                    eos: true,
                    epoch,
                };
                if !push_with_retry(inner, &inner.video_q, eos) {
                    return Ok(SessionEnd::Stopped);
                }
            }
            if audio_idx.is_some() {
                let eos = Packet {
                    data: Vec::new(),
                    pts_us: last_pts,
                    flags: 0,
                    eos: true,
                    epoch,
                };
                if !push_with_retry(inner, &inner.audio_q, eos) {
                    return Ok(SessionEnd::Stopped);
                }
            }
            crate::vlog!("demux: end of stream after {samples} samples");
            return Ok(SessionEnd::Eof);
        }

        let n = ex.read_sample(&mut buf);
        if n < 0 {
            return Err(format!(
                "readSampleData failed: {n} ({})",
                crate::platform::ndk::media_status_name(n as i32)
            ));
        }
        let pts = ex.sample_time();
        let flags = ex.sample_flags();
        last_pts = pts.max(last_pts);
        samples += 1;
        let packet = Packet {
            data: buf[..n as usize].to_vec(),
            pts_us: pts,
            flags,
            eos: false,
            epoch,
        };
        let is_video = video_idx.map(|v| track as usize == v).unwrap_or(false);
        let q = if is_video {
            &inner.video_q
        } else {
            &inner.audio_q
        };
        if !push_with_retry(inner, q, packet) {
            return Ok(SessionEnd::Stopped);
        }
        if !ex.advance() {
            continue; // loop will pick up the EOS on the next iteration
        }
    }
}

/// Push a packet, retrying while the queue is full.
///
/// Seek requests are *not* consumed here: the read loop owns the extractor and
/// must be the one to perform the seek, so this simply gives up and lets the
/// loop notice the request.
fn push_with_retry(
    inner: &Arc<Inner>,
    q: &crate::engine::queue::PacketQueue,
    packet: Packet,
) -> bool {
    for _ in 0..8 {
        if !inner.running.load(Ordering::Acquire) || inner.stop_requested.load(Ordering::Acquire) {
            return false;
        }
        if inner.seek_request_ms.load(Ordering::Acquire) >= 0 {
            // a seek is pending: drop this packet, the loop flushes and seeks
            return true;
        }
        if q.push(packet.clone(), PUSH_TIMEOUT) {
            return true;
        }
        if q.is_aborted() {
            return false;
        }
    }
    false
}

/// Run the lightweight container prober over the same source.
fn probe_container(source: &Source) -> Option<vlcrs_media::MediaInfo> {
    let info = match source {
        Source::Uri(uri) => {
            let path = strip_file_scheme(uri);
            if path.is_empty() {
                // network or opaque URI: the platform demuxer will handle it,
                // but we cannot probe metadata without fetching it
                crate::vlog!("skipping metadata probe for {uri}");
                return None;
            }
            match std::fs::File::open(&path) {
                Ok(mut f) => vlcrs_media::probe(&mut f),
                Err(e) => {
                    crate::vwarn!("cannot open {path} for probing: {e}");
                    return None;
                }
            }
        }
        Source::Fd { fd, offset, length } => {
            let dup = unsafe { crate::platform::ndk::dup(*fd) };
            if dup < 0 {
                crate::vwarn!("dup() failed, cannot probe metadata");
                return None;
            }
            let file = unsafe { std::os::unix::io::FromRawFd::from_raw_fd(dup) };
            let mut slice = FdSlice::new(file, *offset, *length);
            vlcrs_media::probe(&mut slice)
        }
    };
    match info {
        Ok(i) => {
            crate::vlog!("probe: {}", i.summary());
            Some(i)
        }
        Err(e) => {
            crate::vlog!("probe failed: {e}");
            None
        }
    }
}

fn strip_file_scheme(uri: &str) -> String {
    if let Some(rest) = uri.strip_prefix("file://") {
        return rest.to_string();
    }
    if uri.contains("://") {
        return String::new();
    }
    uri.to_string()
}

/// A `Read + Seek` window over an open descriptor.
struct FdSlice {
    file: std::fs::File,
    base: u64,
    len: u64,
    pos: u64,
}

impl FdSlice {
    fn new(file: std::fs::File, base: i64, len: i64) -> FdSlice {
        let base = base.max(0) as u64;
        let len = if len > 0 {
            len as u64
        } else {
            file.metadata()
                .map(|m| m.len().saturating_sub(base))
                .unwrap_or(0)
        };
        FdSlice {
            file,
            base,
            len,
            pos: 0,
        }
    }
}

impl Read for FdSlice {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.len.saturating_sub(self.pos);
        if remaining == 0 {
            return Ok(0);
        }
        let want = out.len().min(remaining as usize);
        self.file.seek(SeekFrom::Start(self.base + self.pos))?;
        let n = self.file.read(&mut out[..want])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for FdSlice {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let target: i64 = match from {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(p) => self.len as i64 + p,
            SeekFrom::Current(p) => self.pos as i64 + p,
        };
        self.pos = target.clamp(0, self.len as i64) as u64;
        Ok(self.pos)
    }
}
