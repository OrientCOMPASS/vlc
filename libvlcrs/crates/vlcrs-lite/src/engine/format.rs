//! Serializable track format.
//!
//! `AMediaFormat` is an opaque, non-`Send` platform object owned by the thread
//! that created it.  The demux thread reads it, copies the interesting keys into
//! this plain Rust struct, and the decode threads rebuild an `AMediaFormat`
//! from it — no pointer hand-off between threads, and the whole format ends up
//! in the log where it can be debugged.

use crate::platform::media::Format;
use crate::platform::ndk::key;

/// Integer keys copied verbatim when present.
const INT_KEYS: [&str; 22] = [
    "profile",
    "level",
    "max-input-size",
    "color-standard",
    "color-transfer",
    "color-range",
    "priority",
    "crop-left",
    "crop-top",
    "crop-right",
    "crop-bottom",
    "bitrate",
    "sample-rate",
    "channel-count",
    "pcm-encoding",
    "aac-is-adts",
    "is-adts",
    "block-align",
    "encoder-delay",
    "aac-profile",
    "width",
    "height",
];

/// Float keys copied verbatim when present.
const FLOAT_KEYS: [&str; 2] = ["frame-rate", "capture-rate"];

/// Buffer keys copied verbatim when present.
const BUFFER_KEYS: [&str; 3] = [key::CSD_0, key::CSD_1, "csd-2"];

/// A track format that can cross thread boundaries.
#[derive(Clone, Debug, Default)]
pub(crate) struct TrackFormat {
    pub mime: String,
    pub width: i32,
    pub height: i32,
    pub rotation: i32,
    pub frame_rate: f32,
    pub sample_rate: i32,
    pub channels: i32,
    pub duration_us: i64,
    pub max_input_size: i32,
    pub csd: Vec<(String, Vec<u8>)>,
    pub ints: Vec<(String, i32)>,
    pub floats: Vec<(String, f32)>,
}

impl TrackFormat {
    /// Snapshot an `AMediaFormat`.
    pub fn from(fmt: &Format) -> TrackFormat {
        let mut t = TrackFormat {
            mime: fmt.get_str(key::MIME).unwrap_or_default(),
            width: fmt.get_i32(key::WIDTH).unwrap_or(0),
            height: fmt.get_i32(key::HEIGHT).unwrap_or(0),
            rotation: fmt.get_i32(key::ROTATION).unwrap_or(0),
            frame_rate: fmt.get_f32("frame-rate").unwrap_or(0.0),
            sample_rate: fmt.get_i32(key::SAMPLE_RATE).unwrap_or(0),
            channels: fmt.get_i32(key::CHANNEL_COUNT).unwrap_or(0),
            duration_us: fmt.get_i64(key::DURATION).unwrap_or(0),
            max_input_size: fmt.get_i32(key::MAX_INPUT_SIZE).unwrap_or(0),
            csd: Vec::new(),
            ints: Vec::new(),
            floats: Vec::new(),
        };
        for k in INT_KEYS {
            if let Some(v) = fmt.get_i32(k) {
                t.ints.push((k.to_string(), v));
            }
        }
        for k in FLOAT_KEYS {
            if let Some(v) = fmt.get_f32(k) {
                t.floats.push((k.to_string(), v));
            }
        }
        for k in BUFFER_KEYS {
            if let Some(b) = fmt.get_buffer(k) {
                t.csd.push((k.to_string(), b));
            }
        }
        t
    }

    /// Rebuild an `AMediaFormat` for `AMediaCodec_configure`.
    pub fn build(&self) -> Option<Format> {
        let mut fmt = Format::new()?;
        if !self.mime.is_empty() {
            fmt.set_str(key::MIME, &self.mime);
        }
        if self.width > 0 {
            fmt.set_i32(key::WIDTH, self.width);
        }
        if self.height > 0 {
            fmt.set_i32(key::HEIGHT, self.height);
        }
        if self.max_input_size > 0 {
            fmt.set_i32(key::MAX_INPUT_SIZE, self.max_input_size);
        }
        for (k, v) in &self.ints {
            fmt.set_i32(k, *v);
        }
        for (k, v) in &self.floats {
            if let Ok(c) = std::ffi::CString::new(k.clone()) {
                unsafe {
                    crate::platform::ndk::AMediaFormat_setFloat(fmt.as_ptr(), c.as_ptr(), *v)
                };
            }
        }
        for (k, data) in &self.csd {
            fmt.set_buffer(k, data);
        }
        Some(fmt)
    }

    /// Compact description for logs.
    pub fn describe(&self) -> String {
        format!(
            "{} {}x{} rot={} fps={:.2} {}Hz/{}ch csd={} ints={}",
            self.mime,
            self.width,
            self.height,
            self.rotation,
            self.frame_rate,
            self.sample_rate,
            self.channels,
            self.csd.len(),
            self.ints.len()
        )
    }

    /// `true` when this looks like a video track.
    pub fn is_video(&self) -> bool {
        crate::platform::media::mime_is_video(&self.mime)
    }

    /// `true` when this looks like an audio track.
    pub fn is_audio(&self) -> bool {
        crate::platform::media::mime_is_audio(&self.mime)
    }
}
