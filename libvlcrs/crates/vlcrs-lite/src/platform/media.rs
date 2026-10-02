//! Safe wrappers around `libmediandk` (`AMediaExtractor`, `AMediaCodec`,
//! `AMediaFormat`).
//!
//! The wrappers are deliberately *not* `Send`: each one is owned by exactly one
//! engine thread, which is the lifetime model `MediaCodec` expects.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

use super::ndk::*;

/// An `AMediaFormat`.
pub struct Format {
    ptr: *mut AMediaFormat,
    owned: bool,
}

impl Format {
    /// Wrap a format the caller owns (it will *not* be deleted).
    ///
    /// # Safety
    /// `ptr` must be a valid `AMediaFormat*` or null.
    pub unsafe fn borrowed(ptr: *mut AMediaFormat) -> Option<Format> {
        if ptr.is_null() {
            None
        } else {
            Some(Format { ptr, owned: false })
        }
    }

    /// Wrap a format we own (deleted on drop).
    ///
    /// # Safety
    /// `ptr` must be a valid, owned `AMediaFormat*` or null.
    pub unsafe fn owned(ptr: *mut AMediaFormat) -> Option<Format> {
        if ptr.is_null() {
            None
        } else {
            Some(Format { ptr, owned: true })
        }
    }

    /// Create an empty format.
    pub fn new() -> Option<Format> {
        unsafe { Format::owned(AMediaFormat_new()) }
    }

    /// Raw pointer, for the NDK calls that take one.
    pub fn as_ptr(&self) -> *mut AMediaFormat {
        self.ptr
    }

    /// Read a string value.
    pub fn get_str(&self, key: &str) -> Option<String> {
        let ckey = CString::new(key).ok()?;
        let mut out: *const c_char = ptr::null();
        let ok = unsafe { AMediaFormat_getString(self.ptr, ckey.as_ptr(), &mut out) };
        if !ok || out.is_null() {
            return None;
        }
        unsafe { CStr::from_ptr(out) }
            .to_str()
            .ok()
            .map(String::from)
    }

    /// Read an `int32` value.
    pub fn get_i32(&self, key: &str) -> Option<i32> {
        let ckey = CString::new(key).ok()?;
        let mut out = 0i32;
        let ok = unsafe { AMediaFormat_getInt32(self.ptr, ckey.as_ptr(), &mut out) };
        if ok {
            Some(out)
        } else {
            None
        }
    }

    /// Read an `int64` value.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        let ckey = CString::new(key).ok()?;
        let mut out = 0i64;
        let ok = unsafe { AMediaFormat_getInt64(self.ptr, ckey.as_ptr(), &mut out) };
        if ok {
            Some(out)
        } else {
            None
        }
    }

    /// Read a float value.
    pub fn get_f32(&self, key: &str) -> Option<f32> {
        let ckey = CString::new(key).ok()?;
        let mut out = 0f32;
        let ok = unsafe { AMediaFormat_getFloat(self.ptr, ckey.as_ptr(), &mut out) };
        if ok {
            Some(out)
        } else {
            None
        }
    }

    /// Read a buffer value (copied).
    pub fn get_buffer(&self, key: &str) -> Option<Vec<u8>> {
        let ckey = CString::new(key).ok()?;
        let mut data: *mut std::os::raw::c_void = ptr::null_mut();
        let mut size = 0usize;
        let ok = unsafe { AMediaFormat_getBuffer(self.ptr, ckey.as_ptr(), &mut data, &mut size) };
        if !ok || data.is_null() || size == 0 || size > 64 * 1024 * 1024 {
            return None;
        }
        let mut out = vec![0u8; size];
        unsafe { ptr::copy_nonoverlapping(data as *const u8, out.as_mut_ptr(), size) };
        Some(out)
    }

    /// Set an `int32` value.
    pub fn set_i32(&mut self, key: &str, value: i32) {
        if let Ok(ckey) = CString::new(key) {
            unsafe { AMediaFormat_setInt32(self.ptr, ckey.as_ptr(), value) };
        }
    }

    /// Set a string value.
    pub fn set_str(&mut self, key: &str, value: &str) {
        if let (Ok(ckey), Ok(cval)) = (CString::new(key), CString::new(value)) {
            unsafe { AMediaFormat_setString(self.ptr, ckey.as_ptr(), cval.as_ptr()) };
        }
    }

    /// Set a buffer value (copied by the platform).
    pub fn set_buffer(&mut self, key: &str, data: &[u8]) {
        if let Ok(ckey) = CString::new(key) {
            unsafe {
                AMediaFormat_setBuffer(
                    self.ptr,
                    ckey.as_ptr(),
                    data.as_ptr() as *const std::os::raw::c_void,
                    data.len(),
                )
            };
        }
    }

    /// Human readable dump (uses `AMediaFormat_toString`).
    pub fn describe(&self) -> String {
        let s = unsafe { AMediaFormat_toString(self.ptr) };
        if s.is_null() {
            return "<null format>".to_string();
        }
        unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
    }
}

impl Drop for Format {
    fn drop(&mut self) {
        if self.owned && !self.ptr.is_null() {
            unsafe { AMediaFormat_delete(self.ptr) };
        }
    }
}

/// `AMediaExtractor`.
pub struct Extractor {
    ptr: *mut AMediaExtractor,
}

impl Extractor {
    /// Create a new extractor.
    pub fn new() -> Option<Extractor> {
        let ptr = unsafe { AMediaExtractor_new() };
        if ptr.is_null() {
            None
        } else {
            Some(Extractor { ptr })
        }
    }

    /// Set a URI/path data source (`file://…`, `http://…`, plain path).
    pub fn set_data_source(&mut self, uri: &str) -> MediaStatus {
        let Ok(c) = CString::new(uri) else {
            return AMEDIA_ERROR_INVALID_PARAMETER;
        };
        unsafe { AMediaExtractor_setDataSource(self.ptr, c.as_ptr()) }
    }

    /// Set a file descriptor data source (content URIs).
    pub fn set_data_source_fd(&mut self, fd: i32, offset: i64, length: i64) -> MediaStatus {
        unsafe { AMediaExtractor_setDataSourceFd(self.ptr, fd, offset, length) }
    }

    /// Number of tracks.
    pub fn track_count(&self) -> usize {
        unsafe { AMediaExtractor_getTrackCount(self.ptr) }
    }

    /// Track format (borrowed; deleted here because the NDK returns a new
    /// object — we take ownership).
    pub fn track_format(&self, idx: usize) -> Option<Format> {
        unsafe { Format::owned(AMediaExtractor_getTrackFormat(self.ptr, idx)) }
    }

    /// Select a track for reading.
    pub fn select_track(&mut self, idx: usize) -> MediaStatus {
        unsafe { AMediaExtractor_selectTrack(self.ptr, idx) }
    }

    /// Unselect a track.
    pub fn unselect_track(&mut self, idx: usize) -> MediaStatus {
        unsafe { AMediaExtractor_unselectTrack(self.ptr, idx) }
    }

    /// Read the current sample into `buf`, returning its size, or a negative
    /// error code.
    pub fn read_sample(&mut self, buf: &mut [u8]) -> isize {
        unsafe { AMediaExtractor_readSampleData(self.ptr, buf.as_mut_ptr(), buf.len()) }
    }

    /// Flags of the current sample.
    pub fn sample_flags(&self) -> u32 {
        unsafe { AMediaExtractor_getSampleFlags(self.ptr) }
    }

    /// Track index of the current sample, `-1` when there is none.
    pub fn sample_track_index(&self) -> i32 {
        unsafe { AMediaExtractor_getSampleTrackIndex(self.ptr) }
    }

    /// Presentation time of the current sample, in microseconds.
    pub fn sample_time(&self) -> i64 {
        unsafe { AMediaExtractor_getSampleTime(self.ptr) }
    }

    /// Move to the next sample. `false` at end of stream.
    pub fn advance(&mut self) -> bool {
        unsafe { AMediaExtractor_advance(self.ptr) }
    }

    /// Seek, times in microseconds.
    pub fn seek_to(&mut self, us: i64, mode: i32) -> MediaStatus {
        unsafe { AMediaExtractor_seekTo(self.ptr, us, mode) }
    }
}

impl Drop for Extractor {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { AMediaExtractor_delete(self.ptr) };
        }
    }
}

/// Result of `dequeueOutputBuffer`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dequeue {
    /// An output buffer is available.
    Buffer(usize, BufferInfo),
    /// Try again later.
    TryAgain,
    /// The output format changed.
    FormatChanged,
    /// Legacy notification, ignore.
    BuffersChanged,
    /// An error occurred.
    Error(MediaStatus),
}

/// Normalised buffer info.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BufferInfo {
    /// Offset inside the buffer.
    pub offset: i32,
    /// Size in bytes.
    pub size: i32,
    /// Presentation time in microseconds.
    pub pts_us: i64,
    /// Codec buffer flags.
    pub flags: u32,
}

impl BufferInfo {
    /// `true` for the end-of-stream marker.
    pub fn is_eos(&self) -> bool {
        self.flags & buffer_flag::END_OF_STREAM != 0
    }
    /// `true` for codec configuration data (must not be rendered).
    pub fn is_codec_config(&self) -> bool {
        self.flags & buffer_flag::CODEC_CONFIG != 0
    }
}

/// `AMediaCodec`.
pub struct Codec {
    ptr: *mut AMediaCodec,
}

impl Codec {
    /// Create a decoder for a MIME type.
    pub fn create_decoder(mime: &str) -> Option<Codec> {
        let c = CString::new(mime).ok()?;
        let ptr = unsafe { AMediaCodec_createDecoderByType(c.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(Codec { ptr })
        }
    }

    /// Create a codec by its component name.
    pub fn create_by_name(name: &str) -> Option<Codec> {
        let c = CString::new(name).ok()?;
        let ptr = unsafe { AMediaCodec_createCodecByName(c.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(Codec { ptr })
        }
    }

    /// Configure the codec; `surface` may be null for byte buffer output.
    pub fn configure(
        &mut self,
        format: &Format,
        surface: *mut ANativeWindow,
        flags: u32,
    ) -> MediaStatus {
        unsafe { AMediaCodec_configure(self.ptr, format.as_ptr(), surface, ptr::null_mut(), flags) }
    }

    /// Start the codec.
    pub fn start(&mut self) -> MediaStatus {
        unsafe { AMediaCodec_start(self.ptr) }
    }

    /// Stop the codec.
    pub fn stop(&mut self) -> MediaStatus {
        unsafe { AMediaCodec_stop(self.ptr) }
    }

    /// Flush the codec (all pending buffers are discarded).
    pub fn flush(&mut self) -> MediaStatus {
        unsafe { AMediaCodec_flush(self.ptr) }
    }

    /// Dequeue an input buffer index.
    pub fn dequeue_input_buffer(&mut self, timeout_us: i64) -> Option<usize> {
        let idx = unsafe { AMediaCodec_dequeueInputBuffer(self.ptr, timeout_us) };
        if idx >= 0 {
            Some(idx as usize)
        } else {
            None
        }
    }

    /// Pointer and capacity of an input buffer.
    ///
    /// # Safety
    /// The returned pointer is only valid until the buffer is queued.
    pub unsafe fn input_buffer(&self, idx: usize) -> Option<(*mut u8, usize)> {
        let mut size = 0usize;
        let p = unsafe { AMediaCodec_getInputBuffer(self.ptr, idx, &mut size) };
        if p.is_null() || size == 0 {
            None
        } else {
            Some((p, size))
        }
    }

    /// Pointer and capacity of an output buffer.
    ///
    /// # Safety
    /// The returned pointer is only valid until the buffer is released.
    pub unsafe fn output_buffer(&self, idx: usize) -> Option<(*const u8, usize)> {
        let mut size = 0usize;
        let p = unsafe { AMediaCodec_getOutputBuffer(self.ptr, idx, &mut size) };
        if p.is_null() || size == 0 {
            None
        } else {
            Some((p as *const u8, size))
        }
    }

    /// Copy `data` into an input buffer and queue it.
    pub fn queue_input(&mut self, idx: usize, data: &[u8], pts_us: i64, flags: u32) -> MediaStatus {
        let (p, cap) = match unsafe { self.input_buffer(idx) } {
            Some(v) => v,
            None => return AMEDIA_ERROR_INVALID_PARAMETER,
        };
        if data.len() > cap {
            return AMEDIA_ERROR_INVALID_PARAMETER;
        }
        unsafe { ptr::copy_nonoverlapping(data.as_ptr(), p, data.len()) };
        unsafe {
            AMediaCodec_queueInputBuffer(self.ptr, idx, 0, data.len(), pts_us.max(0) as u64, flags)
        }
    }

    /// Queue an empty buffer with the end-of-stream flag.
    pub fn queue_eos(&mut self, idx: usize, pts_us: i64) -> MediaStatus {
        unsafe {
            AMediaCodec_queueInputBuffer(
                self.ptr,
                idx,
                0,
                0,
                pts_us.max(0) as u64,
                buffer_flag::END_OF_STREAM,
            )
        }
    }

    /// Dequeue an output buffer.
    pub fn dequeue_output_buffer(&mut self, timeout_us: i64) -> Dequeue {
        let mut info = AMediaCodecBufferInfo::default();
        let idx = unsafe { AMediaCodec_dequeueOutputBuffer(self.ptr, &mut info, timeout_us) };
        match idx {
            i if i >= 0 => Dequeue::Buffer(
                i as usize,
                BufferInfo {
                    offset: info.offset,
                    size: info.size,
                    pts_us: info.presentation_time_us,
                    flags: info.flags,
                },
            ),
            info::TRY_AGAIN_LATER => Dequeue::TryAgain,
            info::OUTPUT_FORMAT_CHANGED => Dequeue::FormatChanged,
            info::OUTPUT_BUFFERS_CHANGED => Dequeue::BuffersChanged,
            other => Dequeue::Error(other as MediaStatus),
        }
    }

    /// Release an output buffer, optionally rendering it to the surface.
    pub fn release_output_buffer(&mut self, idx: usize, render: bool) -> MediaStatus {
        unsafe { AMediaCodec_releaseOutputBuffer(self.ptr, idx, render) }
    }

    /// Current output format.
    pub fn output_format(&self) -> Option<Format> {
        unsafe { Format::owned(AMediaCodec_getOutputFormat(self.ptr)) }
    }

    /// Component name (API 28+; `None` on older platforms).
    pub fn name(&self) -> Option<String> {
        let mut raw: *mut c_char = ptr::null_mut();
        let st = unsafe { AMediaCodec_getName(self.ptr, &mut raw) };
        if st != AMEDIA_OK || raw.is_null() {
            return None;
        }
        let name = unsafe { CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        unsafe { AMediaCodec_releaseName(self.ptr, raw) };
        Some(name)
    }

    /// Whether an error action code is recoverable.
    pub fn is_recoverable(action_code: i32) -> bool {
        unsafe { AMediaCodecActionCode_isRecoverable(action_code) }
    }
}

impl Drop for Codec {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                AMediaCodec_stop(self.ptr);
                AMediaCodec_delete(self.ptr);
            };
        }
    }
}

/// `true` for `video/*` MIME types.
pub fn mime_is_video(mime: &str) -> bool {
    mime.starts_with("video/")
}

/// `true` for `audio/*` MIME types.
pub fn mime_is_audio(mime: &str) -> bool {
    mime.starts_with("audio/")
}

/// Set the thread name (visible in `systrace`/`top`).
pub fn set_thread_name(name: &str) {
    if let Ok(c) = CString::new(name) {
        let bytes = c.into_bytes_with_nul();
        // PR_SET_NAME takes at most 16 bytes including the terminator.
        let mut buf = [0u8; 16];
        let n = bytes.len().min(16);
        buf[..n].copy_from_slice(&bytes[..n]);
        buf[15] = 0;
        unsafe {
            prctl(PR_SET_NAME, buf.as_ptr() as u64, 0, 0, 0);
        }
    }
}
