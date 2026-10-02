//! Raw FFI declarations for the Android NDK libraries libvlcrs uses.
//!
//! Every signature below was taken from the NDK headers
//! (`media/NdkMediaExtractor.h`, `media/NdkMediaCodec.h`, `media/NdkMediaFormat.h`,
//! `media/NdkMediaError.h`, `aaudio/AAudio.h`, `android/sensor.h`,
//! `android/looper.h`, `android/native_window*.h`) so the ABI matches exactly.
//! Everything is `unsafe`; the safe wrappers live in the sibling modules.
//!
//! `AMEDIAFORMAT_KEY_*` are *data* symbols that only exist from a given API
//! level on, so the format keys are spelled out as string literals instead of
//! being linked against (the values are part of the documented, stable ABI).

#![allow(missing_docs)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::upper_case_acronyms)]

#[allow(unused_imports)] // the raw types back the public constants on every target
use std::os::raw::{c_char, c_int, c_void};

/// `media_status_t`
pub type MediaStatus = i32;

/// Successful NDK media call.
pub const AMEDIA_OK: MediaStatus = 0;
/// Base of the NDK media error codes.
pub const AMEDIA_ERROR_BASE: MediaStatus = -10_000;
/// Unknown error.
pub const AMEDIA_ERROR_UNKNOWN: MediaStatus = AMEDIA_ERROR_BASE;
/// Malformed bitstream.
pub const AMEDIA_ERROR_MALFORMED: MediaStatus = AMEDIA_ERROR_BASE - 1;
/// Unsupported format.
pub const AMEDIA_ERROR_UNSUPPORTED: MediaStatus = AMEDIA_ERROR_BASE - 2;
/// Invalid object.
pub const AMEDIA_ERROR_INVALID_OBJECT: MediaStatus = AMEDIA_ERROR_BASE - 3;
/// Invalid parameter.
pub const AMEDIA_ERROR_INVALID_PARAMETER: MediaStatus = AMEDIA_ERROR_BASE - 4;
/// Invalid operation for the current state.
pub const AMEDIA_ERROR_INVALID_OPERATION: MediaStatus = AMEDIA_ERROR_BASE - 5;
/// End of stream.
pub const AMEDIA_ERROR_END_OF_STREAM: MediaStatus = AMEDIA_ERROR_BASE - 6;
/// I/O failure.
pub const AMEDIA_ERROR_IO: MediaStatus = AMEDIA_ERROR_BASE - 7;
/// Would block.
pub const AMEDIA_ERROR_WOULD_BLOCK: MediaStatus = AMEDIA_ERROR_BASE - 8;
/// Codec ran out of resources (recoverable).
pub const AMEDIACODEC_ERROR_INSUFFICIENT_RESOURCE: MediaStatus = 1100;
/// Codec was reclaimed by the system (recoverable).
pub const AMEDIACODEC_ERROR_RECLAIMED: MediaStatus = 1101;

/// Human readable name of a `media_status_t`.
pub fn media_status_name(s: MediaStatus) -> &'static str {
    match s {
        AMEDIA_OK => "AMEDIA_OK",
        AMEDIA_ERROR_UNKNOWN => "AMEDIA_ERROR_UNKNOWN",
        AMEDIA_ERROR_MALFORMED => "AMEDIA_ERROR_MALFORMED",
        AMEDIA_ERROR_UNSUPPORTED => "AMEDIA_ERROR_UNSUPPORTED",
        AMEDIA_ERROR_INVALID_OBJECT => "AMEDIA_ERROR_INVALID_OBJECT",
        AMEDIA_ERROR_INVALID_PARAMETER => "AMEDIA_ERROR_INVALID_PARAMETER",
        AMEDIA_ERROR_INVALID_OPERATION => "AMEDIA_ERROR_INVALID_OPERATION",
        AMEDIA_ERROR_END_OF_STREAM => "AMEDIA_ERROR_END_OF_STREAM",
        AMEDIA_ERROR_IO => "AMEDIA_ERROR_IO",
        AMEDIA_ERROR_WOULD_BLOCK => "AMEDIA_ERROR_WOULD_BLOCK",
        AMEDIACODEC_ERROR_INSUFFICIENT_RESOURCE => "AMEDIACODEC_ERROR_INSUFFICIENT_RESOURCE",
        AMEDIACODEC_ERROR_RECLAIMED => "AMEDIACODEC_ERROR_RECLAIMED",
        _ => "AMEDIA_ERROR(?)",
    }
}

/// Opaque extractor.
#[repr(C)]
pub struct AMediaExtractor {
    _opaque: [u8; 0],
}
/// Opaque codec.
#[repr(C)]
pub struct AMediaCodec {
    _opaque: [u8; 0],
}
/// Opaque format.
#[repr(C)]
pub struct AMediaFormat {
    _opaque: [u8; 0],
}
/// Opaque crypto (never used, always passed as null).
#[repr(C)]
pub struct AMediaCrypto {
    _opaque: [u8; 0],
}
/// Opaque native window.
#[repr(C)]
pub struct ANativeWindow {
    _opaque: [u8; 0],
}

/// `AMediaCodecBufferInfo` — note the 32 bit offset/size fields.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AMediaCodecBufferInfo {
    /// Offset of the data inside the buffer.
    pub offset: i32,
    /// Size of the data in bytes.
    pub size: i32,
    /// Presentation timestamp in microseconds.
    pub presentation_time_us: i64,
    /// `AMEDIACODEC_BUFFER_FLAG_*`.
    pub flags: u32,
}

/// Codec buffer flags.
pub mod buffer_flag {
    /// Key frame.
    pub const KEY_FRAME: u32 = 1;
    /// Codec configuration data (must not be rendered).
    pub const CODEC_CONFIG: u32 = 2;
    /// End of stream.
    pub const END_OF_STREAM: u32 = 4;
    /// Partial frame.
    pub const PARTIAL_FRAME: u32 = 8;
}

/// Special `dequeueOutputBuffer` return values.
pub mod info {
    /// No buffer available right now.
    pub const TRY_AGAIN_LATER: isize = -1;
    /// The output format changed.
    pub const OUTPUT_FORMAT_CHANGED: isize = -2;
    /// The output buffers changed (legacy, ignored).
    pub const OUTPUT_BUFFERS_CHANGED: isize = -3;
    /// Unknown error.
    pub const UNKNOWN_ERROR: isize = -1000;
}

/// `AMediaExtractor_seekTo` modes.
pub mod seek_mode {
    /// Seek to the previous sync sample.
    pub const PREVIOUS_SYNC: i32 = 0;
    /// Seek to the next sync sample.
    pub const NEXT_SYNC: i32 = 1;
    /// Seek to the closest sync sample.
    pub const CLOSEST_SYNC: i32 = 2;
}

/// Documented `AMediaFormat` keys (stable string values).
pub mod key {
    /// MIME type, e.g. `video/avc`.
    pub const MIME: &str = "mime";
    /// Picture width.
    pub const WIDTH: &str = "width";
    /// Picture height.
    pub const HEIGHT: &str = "height";
    /// Duration in microseconds.
    pub const DURATION: &str = "durationUs";
    /// Encoder stride.
    pub const STRIDE: &str = "stride";
    /// Encoder slice height.
    pub const SLICE_HEIGHT: &str = "slice-height";
    /// Colour format.
    pub const COLOR_FORMAT: &str = "color-format";
    /// Audio channel count.
    pub const CHANNEL_COUNT: &str = "channel-count";
    /// Audio sample rate.
    pub const SAMPLE_RATE: &str = "sample-rate";
    /// Maximum input buffer size.
    pub const MAX_INPUT_SIZE: &str = "max-input-size";
    /// Bit rate.
    pub const BIT_RATE: &str = "bitrate";
    /// Frame rate.
    pub const FRAME_RATE: &str = "frame-rate";
    /// Codec specific data #0.
    pub const CSD_0: &str = "csd-0";
    /// Codec specific data #1.
    pub const CSD_1: &str = "csd-1";
    /// Display rotation (API 28+, read opportunistically).
    pub const ROTATION: &str = "rotation";
    /// PCM encoding (API 28+).
    pub const PCM_ENCODING: &str = "pcm-encoding";
    /// Profile (audio).
    pub const PROFILE: &str = "profile";
    /// Crop rectangle.
    pub const CROP_LEFT: &str = "crop-left";
    /// Crop rectangle.
    pub const CROP_TOP: &str = "crop-top";
    /// Crop rectangle.
    pub const CROP_RIGHT: &str = "crop-right";
    /// Crop rectangle.
    pub const CROP_BOTTOM: &str = "crop-bottom";
}

#[cfg(target_os = "android")]
#[link(name = "mediandk")]
extern "C" {
    // ---------------- libmediandk: extractor ----------------
    pub fn AMediaExtractor_new() -> *mut AMediaExtractor;
    pub fn AMediaExtractor_delete(ex: *mut AMediaExtractor) -> MediaStatus;
    pub fn AMediaExtractor_setDataSource(
        ex: *mut AMediaExtractor,
        location: *const c_char,
    ) -> MediaStatus;
    pub fn AMediaExtractor_setDataSourceFd(
        ex: *mut AMediaExtractor,
        fd: c_int,
        offset: i64,
        length: i64,
    ) -> MediaStatus;
    pub fn AMediaExtractor_getTrackCount(ex: *mut AMediaExtractor) -> usize;
    pub fn AMediaExtractor_getTrackFormat(
        ex: *mut AMediaExtractor,
        idx: usize,
    ) -> *mut AMediaFormat;
    pub fn AMediaExtractor_selectTrack(ex: *mut AMediaExtractor, idx: usize) -> MediaStatus;
    pub fn AMediaExtractor_unselectTrack(ex: *mut AMediaExtractor, idx: usize) -> MediaStatus;
    pub fn AMediaExtractor_readSampleData(
        ex: *mut AMediaExtractor,
        buffer: *mut u8,
        capacity: usize,
    ) -> isize;
    pub fn AMediaExtractor_getSampleFlags(ex: *mut AMediaExtractor) -> u32;
    pub fn AMediaExtractor_getSampleTrackIndex(ex: *mut AMediaExtractor) -> c_int;
    pub fn AMediaExtractor_getSampleTime(ex: *mut AMediaExtractor) -> i64;
    pub fn AMediaExtractor_advance(ex: *mut AMediaExtractor) -> bool;
    pub fn AMediaExtractor_seekTo(
        ex: *mut AMediaExtractor,
        seek_pos_us: i64,
        mode: i32,
    ) -> MediaStatus;

    // ---------------- libmediandk: format ----------------
    pub fn AMediaFormat_new() -> *mut AMediaFormat;
    pub fn AMediaFormat_delete(fmt: *mut AMediaFormat) -> bool;
    pub fn AMediaFormat_toString(fmt: *mut AMediaFormat) -> *const c_char;
    pub fn AMediaFormat_getInt32(
        fmt: *mut AMediaFormat,
        name: *const c_char,
        out: *mut i32,
    ) -> bool;
    pub fn AMediaFormat_getInt64(
        fmt: *mut AMediaFormat,
        name: *const c_char,
        out: *mut i64,
    ) -> bool;
    pub fn AMediaFormat_getFloat(
        fmt: *mut AMediaFormat,
        name: *const c_char,
        out: *mut f32,
    ) -> bool;
    pub fn AMediaFormat_getSize(
        fmt: *mut AMediaFormat,
        name: *const c_char,
        out: *mut usize,
    ) -> bool;
    pub fn AMediaFormat_getBuffer(
        fmt: *mut AMediaFormat,
        name: *const c_char,
        data: *mut *mut c_void,
        size: *mut usize,
    ) -> bool;
    pub fn AMediaFormat_getString(
        fmt: *mut AMediaFormat,
        name: *const c_char,
        out: *mut *const c_char,
    ) -> bool;
    pub fn AMediaFormat_setInt32(fmt: *mut AMediaFormat, name: *const c_char, value: i32);
    pub fn AMediaFormat_setInt64(fmt: *mut AMediaFormat, name: *const c_char, value: i64);
    pub fn AMediaFormat_setFloat(fmt: *mut AMediaFormat, name: *const c_char, value: f32);
    pub fn AMediaFormat_setString(
        fmt: *mut AMediaFormat,
        name: *const c_char,
        value: *const c_char,
    );
    pub fn AMediaFormat_setBuffer(
        fmt: *mut AMediaFormat,
        name: *const c_char,
        data: *const c_void,
        size: usize,
    );

    // ---------------- libmediandk: codec ----------------
    pub fn AMediaCodec_createDecoderByType(mime: *const c_char) -> *mut AMediaCodec;
    pub fn AMediaCodec_createCodecByName(name: *const c_char) -> *mut AMediaCodec;
    pub fn AMediaCodec_delete(codec: *mut AMediaCodec) -> MediaStatus;
    pub fn AMediaCodec_configure(
        codec: *mut AMediaCodec,
        format: *const AMediaFormat,
        surface: *mut ANativeWindow,
        crypto: *mut AMediaCrypto,
        flags: u32,
    ) -> MediaStatus;
    pub fn AMediaCodec_start(codec: *mut AMediaCodec) -> MediaStatus;
    pub fn AMediaCodec_stop(codec: *mut AMediaCodec) -> MediaStatus;
    pub fn AMediaCodec_flush(codec: *mut AMediaCodec) -> MediaStatus;
    pub fn AMediaCodec_getInputBuffer(
        codec: *mut AMediaCodec,
        idx: usize,
        out_size: *mut usize,
    ) -> *mut u8;
    pub fn AMediaCodec_getOutputBuffer(
        codec: *mut AMediaCodec,
        idx: usize,
        out_size: *mut usize,
    ) -> *mut u8;
    pub fn AMediaCodec_dequeueInputBuffer(codec: *mut AMediaCodec, timeout_us: i64) -> isize;
    pub fn AMediaCodec_queueInputBuffer(
        codec: *mut AMediaCodec,
        idx: usize,
        offset: i64,
        size: usize,
        time: u64,
        flags: u32,
    ) -> MediaStatus;
    pub fn AMediaCodec_dequeueOutputBuffer(
        codec: *mut AMediaCodec,
        info: *mut AMediaCodecBufferInfo,
        timeout_us: i64,
    ) -> isize;
    pub fn AMediaCodec_releaseOutputBuffer(
        codec: *mut AMediaCodec,
        idx: usize,
        render: bool,
    ) -> MediaStatus;
    pub fn AMediaCodec_releaseOutputBufferAtTime(
        codec: *mut AMediaCodec,
        idx: usize,
        timestamp_ns: i64,
    ) -> MediaStatus;
    pub fn AMediaCodec_getOutputFormat(codec: *mut AMediaCodec) -> *mut AMediaFormat;
    pub fn AMediaCodec_setOutputSurface(
        codec: *mut AMediaCodec,
        surface: *mut ANativeWindow,
    ) -> MediaStatus;
    pub fn AMediaCodec_getName(codec: *mut AMediaCodec, out_name: *mut *mut c_char) -> MediaStatus;
    pub fn AMediaCodec_releaseName(codec: *mut AMediaCodec, name: *mut c_char);
    pub fn AMediaCodecActionCode_isRecoverable(action_code: i32) -> bool;
    pub fn AMediaCodecActionCode_isTransient(action_code: i32) -> bool;

}

#[cfg(target_os = "android")]
#[link(name = "android")]
extern "C" {
    // ---------------- libandroid: native window ----------------
    pub fn ANativeWindow_fromSurface(env: *mut c_void, surface: *mut c_void) -> *mut ANativeWindow;
    pub fn ANativeWindow_acquire(window: *mut ANativeWindow);
    pub fn ANativeWindow_release(window: *mut ANativeWindow);
    pub fn ANativeWindow_getWidth(window: *mut ANativeWindow) -> i32;
    pub fn ANativeWindow_getHeight(window: *mut ANativeWindow) -> i32;
    pub fn ANativeWindow_setBuffersGeometry(
        window: *mut ANativeWindow,
        width: i32,
        height: i32,
        format: i32,
    ) -> i32;

    // ---------------- libandroid: sensors + looper ----------------
    pub fn ASensorManager_getInstanceForPackage(package: *const c_char) -> *mut c_void;
    pub fn ASensorManager_getDefaultSensor(mgr: *mut c_void, sensor_type: c_int) -> *const c_void;
    pub fn ASensorManager_createEventQueue(
        mgr: *mut c_void,
        looper: *mut c_void,
        ident: c_int,
        callback: *mut c_void,
        data: *mut c_void,
    ) -> *mut c_void;
    pub fn ASensorManager_destroyEventQueue(mgr: *mut c_void, queue: *mut c_void) -> c_int;
    pub fn ASensorEventQueue_enableSensor(queue: *mut c_void, sensor: *const c_void) -> c_int;
    pub fn ASensorEventQueue_disableSensor(queue: *mut c_void, sensor: *const c_void) -> c_int;
    pub fn ASensorEventQueue_setEventRate(
        queue: *mut c_void,
        sensor: *const c_void,
        usec: i32,
    ) -> c_int;
    pub fn ASensorEventQueue_hasEvents(queue: *mut c_void) -> c_int;
    pub fn ASensorEventQueue_getEvents(
        queue: *mut c_void,
        events: *mut ASensorEvent,
        count: usize,
    ) -> isize;
    pub fn ASensor_getMinDelay(sensor: *const c_void) -> c_int;
    pub fn ASensor_getName(sensor: *const c_void) -> *const c_char;
    pub fn ALooper_prepare(opts: c_int) -> *mut c_void;
    pub fn ALooper_pollAll(
        timeout_millis: c_int,
        out_fd: *mut c_int,
        out_events: *mut c_int,
        out_data: *mut *mut c_void,
    ) -> c_int;
    pub fn ALooper_pollOnce(
        timeout_millis: c_int,
        out_fd: *mut c_int,
        out_events: *mut c_int,
        out_data: *mut *mut c_void,
    ) -> c_int;
    pub fn ALooper_wake(looper: *mut c_void);
    pub fn ALooper_release(looper: *mut c_void);

}

#[cfg(target_os = "android")]
#[link(name = "log")]
extern "C" {
    // ---------------- liblog ----------------
    pub fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    pub fn __android_log_print(prio: c_int, tag: *const c_char, fmt: *const c_char, ...) -> c_int;
}

#[cfg(target_os = "android")]
extern "C" {
    // ---------------- libc bits we need ----------------
    pub fn prctl(option: c_int, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> c_int;
    pub fn dup(fd: c_int) -> c_int;
    pub fn close(fd: c_int) -> c_int;
}

/// `PR_SET_NAME`
pub const PR_SET_NAME: c_int = 15;

/// Sensor types.
pub mod sensor_type {
    /// Accelerometer.
    pub const ACCELEROMETER: i32 = 1;
    /// Gyroscope.
    pub const GYROSCOPE: i32 = 4;
    /// Game rotation vector.
    pub const GAME_ROTATION_VECTOR: i32 = 15;
}

/// `ALOOPER_PREPARE_ALLOW_NON_CALLBACKS`
pub const ALOOPER_PREPARE_ALLOW_NON_CALLBACKS: c_int = 1;
/// `ALOOPER_POLL_CALLBACK`
pub const ALOOPER_POLL_CALLBACK: c_int = -2;
/// `ALOOPER_POLL_TIMEOUT`
pub const ALOOPER_POLL_TIMEOUT: c_int = -3;
/// `ALOOPER_POLL_ERROR`
pub const ALOOPER_POLL_ERROR: c_int = -4;

/// `ASensorEvent` — the union is 64 bytes (`float data[16]` / `uint64 data[8]`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ASensorEvent {
    /// Structure version.
    pub version: i32,
    /// Sensor id.
    pub sensor: i32,
    /// Sensor type.
    pub sensor_type: i32,
    /// Unused.
    pub reserved0: i32,
    /// Event timestamp in nanoseconds (`CLOCK_MONOTONIC`).
    pub timestamp: i64,
    /// Payload; the first three floats are x/y/z for vector sensors.
    pub data: [f32; 16],
    /// Event flags.
    pub flags: u32,
    /// Reserved.
    pub reserved1: [i32; 3],
}

impl Default for ASensorEvent {
    fn default() -> Self {
        ASensorEvent {
            version: 0,
            sensor: 0,
            sensor_type: 0,
            reserved0: 0,
            timestamp: 0,
            data: [0.0; 16],
            flags: 0,
            reserved1: [0; 3],
        }
    }
}

/// AAudio result codes.
pub mod aaudio {
    /// Success.
    pub const OK: i32 = 0;
    /// Stream disconnected (device change).
    pub const ERROR_DISCONNECTED: i32 = -899;
    /// Illegal argument.
    pub const ERROR_ILLEGAL_ARGUMENT: i32 = -898;
    /// Internal error.
    pub const ERROR_INTERNAL: i32 = -896;
    /// Invalid state.
    pub const ERROR_INVALID_STATE: i32 = -895;
    /// Invalid handle.
    pub const ERROR_INVALID_HANDLE: i32 = -892;
    /// Unimplemented.
    pub const ERROR_UNIMPLEMENTED: i32 = -890;
    /// Unavailable.
    pub const ERROR_UNAVAILABLE: i32 = -889;
    /// No free handles.
    pub const ERROR_NO_FREE_HANDLES: i32 = -888;
    /// Out of memory.
    pub const ERROR_NO_MEMORY: i32 = -887;
    /// Null pointer.
    pub const ERROR_NULL: i32 = -886;
    /// Timeout.
    pub const ERROR_TIMEOUT: i32 = -885;
    /// Would block.
    pub const ERROR_WOULD_BLOCK: i32 = -884;
    /// Invalid format.
    pub const ERROR_INVALID_FORMAT: i32 = -883;
    /// Out of range.
    pub const ERROR_OUT_OF_RANGE: i32 = -882;
    /// No service.
    pub const ERROR_NO_SERVICE: i32 = -881;
    /// Invalid rate.
    pub const ERROR_INVALID_RATE: i32 = -880;

    /// Output direction.
    pub const DIRECTION_OUTPUT: i32 = 0;
    /// 16 bit integer PCM.
    pub const FORMAT_PCM_I16: i32 = 1;
    /// 32 bit float PCM.
    pub const FORMAT_PCM_FLOAT: i32 = 2;
    /// Shared mode.
    pub const SHARING_MODE_SHARED: i32 = 1;
    /// Exclusive mode.
    pub const SHARING_MODE_EXCLUSIVE: i32 = 0;
    /// Default performance mode.
    pub const PERFORMANCE_MODE_NONE: i32 = 10;
    /// Power saving.
    pub const PERFORMANCE_MODE_POWER_SAVING: i32 = 11;
    /// Low latency.
    pub const PERFORMANCE_MODE_LOW_LATENCY: i32 = 12;
    /// Media usage.
    pub const USAGE_MEDIA: i32 = 1;
    /// Music content.
    pub const CONTENT_TYPE_MUSIC: i32 = 2;

    /// Stream states.
    pub const STATE_UNINITIALIZED: i32 = 0;
    /// Unknown state.
    pub const STATE_UNKNOWN: i32 = 1;
    /// Open.
    pub const STATE_OPEN: i32 = 2;
    /// Starting.
    pub const STATE_STARTING: i32 = 3;
    /// Started.
    pub const STATE_STARTED: i32 = 4;
    /// Pausing.
    pub const STATE_PAUSING: i32 = 5;
    /// Paused.
    pub const STATE_PAUSED: i32 = 6;
    /// Stopping.
    pub const STATE_STOPPING: i32 = 7;
    /// Stopped.
    pub const STATE_STOPPED: i32 = 8;
    /// Flushing.
    pub const STATE_FLUSHING: i32 = 9;
    /// Flushed.
    pub const STATE_FLUSHED: i32 = 10;
    /// Closing.
    pub const STATE_CLOSING: i32 = 11;
    /// Closed.
    pub const STATE_CLOSED: i32 = 12;
    /// Disconnected.
    pub const STATE_DISCONNECTED: i32 = 13;

    /// Human readable name of a result code.
    pub fn result_name(r: i32) -> &'static str {
        match r {
            OK => "AAUDIO_OK",
            ERROR_DISCONNECTED => "AAUDIO_ERROR_DISCONNECTED",
            ERROR_ILLEGAL_ARGUMENT => "AAUDIO_ERROR_ILLEGAL_ARGUMENT",
            ERROR_INTERNAL => "AAUDIO_ERROR_INTERNAL",
            ERROR_INVALID_STATE => "AAUDIO_ERROR_INVALID_STATE",
            ERROR_INVALID_HANDLE => "AAUDIO_ERROR_INVALID_HANDLE",
            ERROR_UNIMPLEMENTED => "AAUDIO_ERROR_UNIMPLEMENTED",
            ERROR_UNAVAILABLE => "AAUDIO_ERROR_UNAVAILABLE",
            ERROR_NO_FREE_HANDLES => "AAUDIO_ERROR_NO_FREE_HANDLES",
            ERROR_NO_MEMORY => "AAUDIO_ERROR_NO_MEMORY",
            ERROR_NULL => "AAUDIO_ERROR_NULL",
            ERROR_TIMEOUT => "AAUDIO_ERROR_TIMEOUT",
            ERROR_WOULD_BLOCK => "AAUDIO_ERROR_WOULD_BLOCK",
            ERROR_INVALID_FORMAT => "AAUDIO_ERROR_INVALID_FORMAT",
            ERROR_OUT_OF_RANGE => "AAUDIO_ERROR_OUT_OF_RANGE",
            ERROR_NO_SERVICE => "AAUDIO_ERROR_NO_SERVICE",
            ERROR_INVALID_RATE => "AAUDIO_ERROR_INVALID_RATE",
            _ => "AAUDIO_ERROR(?)",
        }
    }
}

#[cfg(target_os = "android")]
#[link(name = "aaudio")]
extern "C" {
    // ---------------- libaaudio ----------------
    pub fn AAudio_createStreamBuilder(builder: *mut *mut c_void) -> i32;
    pub fn AAudioStreamBuilder_setDeviceId(builder: *mut c_void, device_id: i32);
    pub fn AAudioStreamBuilder_setSampleRate(builder: *mut c_void, sample_rate: i32);
    pub fn AAudioStreamBuilder_setChannelCount(builder: *mut c_void, channel_count: i32);
    pub fn AAudioStreamBuilder_setFormat(builder: *mut c_void, format: i32);
    pub fn AAudioStreamBuilder_setSharingMode(builder: *mut c_void, mode: i32);
    pub fn AAudioStreamBuilder_setPerformanceMode(builder: *mut c_void, mode: i32);
    pub fn AAudioStreamBuilder_setDirection(builder: *mut c_void, direction: i32);
    pub fn AAudioStreamBuilder_setBufferCapacityInFrames(builder: *mut c_void, frames: i32);
    pub fn AAudioStreamBuilder_setUsage(builder: *mut c_void, usage: i32);
    pub fn AAudioStreamBuilder_setContentType(builder: *mut c_void, content_type: i32);
    pub fn AAudioStreamBuilder_setInputPreset(builder: *mut c_void, preset: i32);
    pub fn AAudioStreamBuilder_openStream(builder: *mut c_void, stream: *mut *mut c_void) -> i32;
    pub fn AAudioStreamBuilder_delete(builder: *mut c_void);
    pub fn AAudioStream_close(stream: *mut c_void) -> i32;
    pub fn AAudioStream_write(
        stream: *mut c_void,
        buffer: *const c_void,
        num_frames: i32,
        timeout_ns: i64,
    ) -> i32;
    pub fn AAudioStream_requestStart(stream: *mut c_void) -> i32;
    pub fn AAudioStream_requestPause(stream: *mut c_void) -> i32;
    pub fn AAudioStream_requestFlush(stream: *mut c_void) -> i32;
    pub fn AAudioStream_requestStop(stream: *mut c_void) -> i32;
    pub fn AAudioStream_getState(stream: *mut c_void) -> i32;
    pub fn AAudioStream_waitForStateChange(
        stream: *mut c_void,
        input_state: i32,
        next_state: *mut i32,
        timeout_ns: i64,
    ) -> i32;
    pub fn AAudioStream_getSampleRate(stream: *mut c_void) -> i32;
    pub fn AAudioStream_getChannelCount(stream: *mut c_void) -> i32;
    pub fn AAudioStream_getFormat(stream: *mut c_void) -> i32;
    pub fn AAudioStream_getFramesPerBurst(stream: *mut c_void) -> i32;
    pub fn AAudioStream_getXRunCount(stream: *mut c_void) -> i32;
    pub fn AAudioStream_getBufferSizeInFrames(stream: *mut c_void) -> i32;
    pub fn AAudioStream_setBufferSizeInFrames(stream: *mut c_void, frames: i32) -> i32;
    pub fn AAudioStream_getTimestamp(
        stream: *mut c_void,
        clock_id: i32,
        frame_position: *mut i64,
        time_nanoseconds: *mut i64,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout of `AMediaCodecBufferInfo` is part of the ABI: 4 + 4 + 8 + 4
    /// (+4 padding) = 24 bytes, with the timestamp 8 byte aligned.
    #[test]
    fn buffer_info_layout() {
        assert_eq!(std::mem::size_of::<AMediaCodecBufferInfo>(), 24);
        assert_eq!(std::mem::align_of::<AMediaCodecBufferInfo>(), 8);
        let info = AMediaCodecBufferInfo {
            offset: 1,
            size: 2,
            presentation_time_us: 3,
            flags: 4,
        };
        let base = &info as *const _ as usize;
        assert_eq!(&info.offset as *const _ as usize - base, 0);
        assert_eq!(&info.size as *const _ as usize - base, 4);
        assert_eq!(&info.presentation_time_us as *const _ as usize - base, 8);
        assert_eq!(&info.flags as *const _ as usize - base, 16);
    }

    /// `ASensorEvent` must be 104 bytes: the union is 64 bytes wide.
    #[test]
    fn sensor_event_layout() {
        assert_eq!(std::mem::size_of::<ASensorEvent>(), 104);
        let e = ASensorEvent::default();
        let base = &e as *const _ as usize;
        assert_eq!(&e.timestamp as *const _ as usize - base, 16);
        assert_eq!(&e.data as *const _ as usize - base, 24);
        assert_eq!(&e.flags as *const _ as usize - base, 88);
    }

    #[test]
    fn status_names_are_complete() {
        assert_eq!(media_status_name(AMEDIA_OK), "AMEDIA_OK");
        assert_eq!(
            media_status_name(AMEDIACODEC_ERROR_RECLAIMED),
            "AMEDIACODEC_ERROR_RECLAIMED"
        );
        assert_eq!(media_status_name(-12345), "AMEDIA_ERROR(?)");
        assert_eq!(aaudio::result_name(aaudio::OK), "AAUDIO_OK");
        assert_eq!(
            aaudio::result_name(aaudio::ERROR_DISCONNECTED),
            "AAUDIO_ERROR_DISCONNECTED"
        );
    }
}
