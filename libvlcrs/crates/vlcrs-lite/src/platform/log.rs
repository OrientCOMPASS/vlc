//! Logging through Android's logcat (`liblog.so`), with a stderr fallback for
//! host builds (unit tests, tooling).

/// logcat priority levels.
#[allow(dead_code)]
pub mod level {
    /// Verbose.
    pub const VERBOSE: i32 = 2;
    /// Debug.
    pub const DEBUG: i32 = 3;
    /// Info.
    pub const INFO: i32 = 4;
    /// Warning.
    pub const WARN: i32 = 5;
    /// Error.
    pub const ERROR: i32 = 6;
}

/// Tag used for every libvlcrs message.
pub const TAG: &str = "libvlcrs";

#[cfg(target_os = "android")]
extern "C" {
    fn __android_log_write(prio: i32, tag: *const c_char, text: *const c_char) -> i32;
}

#[cfg(target_os = "android")]
use std::ffi::CString;
#[cfg(target_os = "android")]
use std::os::raw::c_char;

/// Write one log line.
pub fn log(prio: i32, msg: &str) {
    #[cfg(target_os = "android")]
    {
        let tag = CString::new(TAG).unwrap_or_default();
        let text = CString::new(msg.replace('\n', " ")).unwrap_or_default();
        unsafe {
            __android_log_write(prio, tag.as_ptr(), text.as_ptr());
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let name = match prio {
            level::ERROR => "E",
            level::WARN => "W",
            level::INFO => "I",
            _ => "D",
        };
        eprintln!("{name}/{TAG}: {msg}");
    }
}

/// Info level.
#[macro_export]
macro_rules! vlog {
    ($($arg:tt)*) => { $crate::platform::log::log($crate::platform::log::level::INFO, &format!($($arg)*)) };
}

/// Warning level.
#[macro_export]
macro_rules! vwarn {
    ($($arg:tt)*) => { $crate::platform::log::log($crate::platform::log::level::WARN, &format!($($arg)*)) };
}

/// Error level.
#[macro_export]
macro_rules! verror {
    ($($arg:tt)*) => { $crate::platform::log::log($crate::platform::log::level::ERROR, &format!($($arg)*)) };
}

/// Debug level.
#[macro_export]
macro_rules! vdebug {
    ($($arg:tt)*) => { $crate::platform::log::log($crate::platform::log::level::DEBUG, &format!($($arg)*)) };
}
