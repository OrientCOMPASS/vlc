//! Audio thread: `AMediaCodec` (audio) → PCM → downmix → AAudio, and the master
//! clock.
//!
//! The output stream is always opened as stereo 16 bit; multichannel sources are
//! folded down by [`vlcrs_vr::downmix`] so the engine never needs a resampler or
//! a mixing library.

use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use vlcrs_vr::downmix::{apply_volume, bytes_to_i16_le, downmix_i16_to_stereo};

use crate::api::{ErrorCode, EventType};
use crate::engine::format::TrackFormat;
use crate::engine::queue::Packet;
use crate::engine::{Inner, EOS_AUDIO};
use crate::platform::audio::AudioOut;
use crate::platform::media::{set_thread_name, Codec, Dequeue};
use crate::platform::ndk::{aaudio, AMEDIA_OK};

const IDLE: Duration = Duration::from_millis(10);
const WRITE_TIMEOUT_NS: i64 = 200_000_000;
/// `AudioFormat.ENCODING_PCM_FLOAT`
const PCM_ENCODING_FLOAT: i32 = 4;

enum AudioEnd {
    Eos,
    Stopped,
}

pub(crate) fn audio_thread(inner: Arc<Inner>) {
    set_thread_name("vlcrs-audio");
    while inner.running.load(Ordering::Acquire) {
        if inner.stop_requested.load(Ordering::Acquire)
            || !inner.media_ready.load(Ordering::Acquire)
        {
            std::thread::sleep(IDLE);
            continue;
        }
        let tf = inner.audio_format.lock().ok().and_then(|g| g.clone());
        let Some(tf) = tf else {
            inner.mark_eos(EOS_AUDIO);
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        match run_audio(&inner, &tf) {
            Ok(AudioEnd::Eos) => {
                inner.mark_eos(EOS_AUDIO);
                // Idle until the item is unloaded or a seek restarts playback.
                let epoch = inner.current_epoch();
                while inner.running.load(Ordering::Acquire)
                    && !inner.stop_requested.load(Ordering::Acquire)
                    && inner.media_ready.load(Ordering::Acquire)
                    && inner.current_epoch() == epoch
                {
                    std::thread::sleep(Duration::from_millis(30));
                }
            }
            Ok(AudioEnd::Stopped) => {}
            Err(e) => {
                crate::verror!("audio session failed: {e}");
                inner.post(
                    EventType::Error,
                    if e.contains("AAudio") {
                        ErrorCode::AudioOutputFailed as i32
                    } else {
                        ErrorCode::AudioDecoderFailed as i32
                    },
                    0,
                );
                inner.mark_eos(EOS_AUDIO);
            }
        }
        std::thread::sleep(IDLE);
    }
    crate::vlog!("audio thread exit");
}

fn run_audio(inner: &Arc<Inner>, tf: &TrackFormat) -> Result<AudioEnd, String> {
    let software = inner
        .options
        .lock()
        .map(|o| o.force_software_decoder)
        .unwrap_or(false);
    let mut codec = if software {
        software_audio_decoder(&tf.mime).and_then(|n| Codec::create_by_name(&n))
    } else {
        None
    };
    if codec.is_none() {
        codec = Codec::create_decoder(&tf.mime);
    }
    let mut codec = codec.ok_or_else(|| format!("no audio decoder for {}", tf.mime))?;
    let fmt = tf.build().ok_or("cannot rebuild the audio format")?;
    // SAFETY: a null surface means byte buffer output (audio).
    let st = unsafe { codec.configure(&fmt, ptr::null_mut(), 0) };
    if st != AMEDIA_OK {
        return Err(format!(
            "audio configure failed: {st} ({})",
            crate::platform::ndk::media_status_name(st)
        ));
    }
    let st = codec.start();
    if st != AMEDIA_OK {
        return Err(format!(
            "audio start failed: {st} ({})",
            crate::platform::ndk::media_status_name(st)
        ));
    }

    let mut sample_rate = if tf.sample_rate > 0 {
        tf.sample_rate
    } else {
        44_100
    };
    let mut channels = tf.channels.clamp(1, 8);
    if channels == 0 {
        channels = 2;
    }
    let mut pcm_encoding = 16;
    let mut out = open_output(sample_rate)?;
    crate::vlog!(
        "audio: {} {}Hz {}ch → AAudio {}Hz/{}ch burst={} ({})",
        tf.mime,
        sample_rate,
        channels,
        out.sample_rate(),
        out.channels(),
        out.frames_per_burst(),
        codec.name().unwrap_or_else(|| "?".into())
    );
    let st = out.start();
    if st != aaudio::OK && st != aaudio::ERROR_INVALID_STATE {
        crate::vwarn!("AAudio start: {}", aaudio::result_name(st));
    }

    let mut epoch = inner.current_epoch();
    let mut input_eos = false;
    let mut output_eos = false;
    let mut pending: Option<Packet> = None;
    let mut paused = false;
    let mut raw: Vec<i16> = Vec::with_capacity(16_384);
    let mut stereo: Vec<i16> = Vec::with_capacity(16_384);

    while inner.running.load(Ordering::Acquire) {
        if inner.stop_requested.load(Ordering::Acquire) {
            let _ = out.stop();
            return Ok(AudioEnd::Stopped);
        }

        // ---- seek / flush ----
        let cur_epoch = inner.current_epoch();
        if cur_epoch != epoch {
            epoch = cur_epoch;
            let _ = codec.flush();
            inner.audio_q.drop_stale(epoch);
            pending = None;
            input_eos = false;
            output_eos = false;
            raw.clear();
            stereo.clear();
            let _ = out.pause();
            let _ = out.flush();
            paused = false;
            let _ = out.start();
            crate::vlog!("audio flushed (epoch {epoch})");
        }

        // ---- pause / resume ----
        let want_pause = inner.paused.load(Ordering::Acquire);
        if want_pause != paused {
            paused = want_pause;
            if paused {
                let _ = out.pause();
            } else {
                let _ = out.start();
            }
        }
        if paused {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }

        // ---- feed the decoder ----
        if !input_eos {
            for _ in 0..8 {
                if pending.is_none() {
                    pending = inner.audio_q.pop(Duration::ZERO);
                }
                let Some(pkt) = pending.take() else { break };
                if pkt.epoch != epoch {
                    continue;
                }
                let Some(idx) = codec.dequeue_input_buffer(0) else {
                    pending = Some(pkt);
                    break;
                };
                let st = if pkt.eos {
                    codec.queue_eos(idx, pkt.pts_us)
                } else {
                    codec.queue_input(idx, &pkt.data, pkt.pts_us, pkt.flags)
                };
                if st != AMEDIA_OK {
                    crate::vwarn!("audio queueInputBuffer failed: {st}");
                }
                if pkt.eos {
                    input_eos = true;
                    break;
                }
            }
        }

        // ---- drain decoded PCM ----
        let mut worked = false;
        for _ in 0..8 {
            match codec.dequeue_output_buffer(0) {
                Dequeue::Buffer(idx, bi) => {
                    worked = true;
                    if bi.is_codec_config() {
                        let _ = codec.release_output_buffer(idx, false);
                        continue;
                    }
                    if bi.is_eos() {
                        output_eos = true;
                        let _ = codec.release_output_buffer(idx, false);
                        break;
                    }
                    match write_pcm(
                        inner,
                        &mut codec,
                        idx,
                        &bi,
                        &mut out,
                        &mut raw,
                        &mut stereo,
                        channels as usize,
                        pcm_encoding,
                        sample_rate,
                    ) {
                        Ok(()) => {}
                        Err(e) => {
                            if e.contains("disconnected") {
                                crate::vwarn!("AAudio disconnected, reopening");
                                out.close();
                                out = open_output(sample_rate)?;
                                let _ = out.start();
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
                Dequeue::FormatChanged => {
                    if let Some(f) = codec.output_format() {
                        crate::vlog!("audio output format: {}", f.describe());
                        let sr = f.get_i32("sample-rate").unwrap_or(sample_rate);
                        let ch = f.get_i32("channel-count").unwrap_or(channels);
                        let enc = f.get_i32("pcm-encoding").unwrap_or(pcm_encoding);
                        let rate_changed = sr > 0 && sr != sample_rate;
                        if sr > 0 {
                            sample_rate = sr;
                        }
                        if ch > 0 {
                            channels = ch.clamp(1, 8);
                        }
                        pcm_encoding = enc;
                        if rate_changed {
                            out.close();
                            out = open_output(sample_rate)?;
                            let _ = out.start();
                        }
                    }
                }
                Dequeue::TryAgain | Dequeue::BuffersChanged => break,
                Dequeue::Error(e) => {
                    crate::vwarn!("audio dequeueOutputBuffer error {e}");
                    if Codec::is_recoverable(e) {
                        return Err(format!("audio decoder error {e}"));
                    }
                    break;
                }
            }
        }

        if output_eos {
            let _ = out.stop();
            crate::vlog!("audio end of stream");
            return Ok(AudioEnd::Eos);
        }
        if !worked {
            std::thread::sleep(Duration::from_millis(3));
        }
    }
    let _ = out.stop();
    Ok(AudioEnd::Stopped)
}

#[allow(clippy::too_many_arguments)]
fn write_pcm(
    inner: &Arc<Inner>,
    codec: &mut Codec,
    idx: usize,
    bi: &crate::platform::media::BufferInfo,
    out: &mut AudioOut,
    raw: &mut Vec<i16>,
    stereo: &mut Vec<i16>,
    channels: usize,
    pcm_encoding: i32,
    sample_rate: i32,
) -> Result<(), String> {
    let Some((p, cap)) = (unsafe { codec.output_buffer(idx) }) else {
        let _ = codec.release_output_buffer(idx, false);
        return Ok(());
    };
    let off = bi.offset.max(0) as usize;
    let len = bi.size.max(0) as usize;
    if len == 0 || off.saturating_add(len) > cap {
        let _ = codec.release_output_buffer(idx, false);
        return Ok(());
    }
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(p.add(off), len) };
    raw.clear();
    if pcm_encoding == PCM_ENCODING_FLOAT {
        raw.reserve(bytes.len() / 4);
        for c in bytes.chunks_exact(4) {
            let f = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            raw.push(float_to_i16(f));
        }
    } else {
        raw.extend(bytes_to_i16_le(bytes));
    }
    let ch = channels.max(1);
    let frames = raw.len() / ch;
    stereo.clear();
    downmix_i16_to_stereo(raw, ch, frames, stereo);
    let volume = inner.volume();
    apply_volume(stereo, volume);

    let total = stereo.len() / 2;
    let mut written = 0usize;
    let rate = sample_rate.max(1) as i64;
    while written < total {
        if !inner.running.load(Ordering::Acquire)
            || inner.stop_requested.load(Ordering::Acquire)
            || inner.paused.load(Ordering::Acquire)
        {
            break;
        }
        let n = out.write(
            &stereo[written * 2..],
            (total - written) as i32,
            WRITE_TIMEOUT_NS,
        );
        if n > 0 {
            written += n as usize;
            inner
                .clock
                .set_audio(bi.pts_us + (written as i64 * 1_000_000) / rate);
            continue;
        }
        match n {
            0 => std::thread::sleep(Duration::from_millis(2)),
            aaudio::ERROR_WOULD_BLOCK | aaudio::ERROR_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(2))
            }
            aaudio::ERROR_DISCONNECTED => {
                let _ = codec.release_output_buffer(idx, false);
                return Err("AAudio disconnected".into());
            }
            other => {
                crate::vwarn!("AAudio write failed: {}", aaudio::result_name(other));
                break;
            }
        }
    }
    if written == 0 {
        inner.stats.audio_underruns.fetch_add(1, Ordering::Relaxed);
    }
    let _ = codec.release_output_buffer(idx, false);
    Ok(())
}

fn float_to_i16(v: f32) -> i16 {
    if !v.is_finite() {
        0
    } else {
        (v * 32767.0).clamp(-32768.0, 32767.0) as i16
    }
}

fn open_output(sample_rate: i32) -> Result<AudioOut, String> {
    AudioOut::open(sample_rate, 2)
        .map_err(|e| format!("AAudio open failed: {e} ({})", aaudio::result_name(e)))
}

fn software_audio_decoder(mime: &str) -> Option<String> {
    let name = match mime {
        "audio/mp4a-latm" => "c2.android.aac.decoder",
        "audio/mpeg" => "c2.android.mp3.decoder",
        "audio/opus" => "c2.android.opus.decoder",
        "audio/vorbis" => "c2.android.vorbis.decoder",
        "audio/flac" => "c2.android.flac.decoder",
        "audio/g711-alaw" => "c2.android.g711.alaw.decoder",
        "audio/g711-mlaw" => "c2.android.g711.mlaw.decoder",
        "audio/raw" => "c2.android.raw.decoder",
        _ => return None,
    };
    Some(name.to_string())
}
