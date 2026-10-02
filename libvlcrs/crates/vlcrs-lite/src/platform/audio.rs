//! AAudio output stream (`libaaudio.so`, API 26+).

use std::os::raw::c_void;
use std::ptr;

use super::ndk::aaudio;
use super::ndk::*;

/// An open AAudio output stream, always 16 bit integer PCM.
pub struct AudioOut {
    stream: *mut c_void,
    sample_rate: i32,
    channels: i32,
}

impl AudioOut {
    /// Open a shared, low-latency-neutral output stream.
    ///
    /// Returns the stream or the AAudio error code.
    pub fn open(sample_rate: i32, channels: i32) -> Result<AudioOut, i32> {
        let mut builder: *mut c_void = ptr::null_mut();
        let r = unsafe { AAudio_createStreamBuilder(&mut builder) };
        if r != aaudio::OK || builder.is_null() {
            return Err(r);
        }
        unsafe {
            AAudioStreamBuilder_setDirection(builder, aaudio::DIRECTION_OUTPUT);
            AAudioStreamBuilder_setSampleRate(builder, sample_rate);
            AAudioStreamBuilder_setChannelCount(builder, channels);
            AAudioStreamBuilder_setFormat(builder, aaudio::FORMAT_PCM_I16);
            AAudioStreamBuilder_setSharingMode(builder, aaudio::SHARING_MODE_SHARED);
            AAudioStreamBuilder_setPerformanceMode(builder, aaudio::PERFORMANCE_MODE_NONE);
            AAudioStreamBuilder_setUsage(builder, aaudio::USAGE_MEDIA);
            AAudioStreamBuilder_setContentType(builder, aaudio::CONTENT_TYPE_MUSIC);
            // A couple of bursts of head room keeps underruns rare without
            // adding audible latency.
            AAudioStreamBuilder_setBufferCapacityInFrames(builder, sample_rate / 5);
        }
        let mut stream: *mut c_void = ptr::null_mut();
        let r = unsafe { AAudioStreamBuilder_openStream(builder, &mut stream) };
        unsafe { AAudioStreamBuilder_delete(builder) };
        if r != aaudio::OK || stream.is_null() {
            return Err(r);
        }
        let actual_rate = unsafe { AAudioStream_getSampleRate(stream) };
        let actual_channels = unsafe { AAudioStream_getChannelCount(stream) };
        Ok(AudioOut {
            stream,
            sample_rate: if actual_rate > 0 {
                actual_rate
            } else {
                sample_rate
            },
            channels: if actual_channels > 0 {
                actual_channels
            } else {
                channels
            },
        })
    }

    /// Effective sample rate (may differ from the request).
    pub fn sample_rate(&self) -> i32 {
        self.sample_rate
    }

    /// Effective channel count.
    pub fn channels(&self) -> i32 {
        self.channels
    }

    /// Frames per burst.
    pub fn frames_per_burst(&self) -> i32 {
        unsafe { AAudioStream_getFramesPerBurst(self.stream) }
    }

    /// Start playback.
    pub fn start(&self) -> i32 {
        unsafe { AAudioStream_requestStart(self.stream) }
    }

    /// Pause playback (keeps the written position).
    pub fn pause(&self) -> i32 {
        unsafe { AAudioStream_requestPause(self.stream) }
    }

    /// Flush a paused stream.
    pub fn flush(&self) -> i32 {
        unsafe { AAudioStream_requestFlush(self.stream) }
    }

    /// Stop playback.
    pub fn stop(&self) -> i32 {
        unsafe { AAudioStream_requestStop(self.stream) }
    }

    /// Current stream state.
    pub fn state(&self) -> i32 {
        unsafe { AAudioStream_getState(self.stream) }
    }

    /// Underrun counter.
    pub fn xrun_count(&self) -> i32 {
        let c = unsafe { AAudioStream_getXRunCount(self.stream) };
        if c < 0 {
            0
        } else {
            c
        }
    }

    /// Blocking write of `frames` interleaved stereo `i16` samples.
    ///
    /// Returns the number of frames written, or a negative AAudio error.
    pub fn write(&self, samples: &[i16], frames: i32, timeout_ns: i64) -> i32 {
        if frames <= 0 || samples.is_empty() {
            return 0;
        }
        unsafe {
            AAudioStream_write(
                self.stream,
                samples.as_ptr() as *const c_void,
                frames,
                timeout_ns,
            )
        }
    }

    /// Close the stream.
    pub fn close(&mut self) {
        if !self.stream.is_null() {
            unsafe {
                AAudioStream_close(self.stream);
            }
            self.stream = ptr::null_mut();
        }
    }
}

impl Drop for AudioOut {
    fn drop(&mut self) {
        self.close();
    }
}
