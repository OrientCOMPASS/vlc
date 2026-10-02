//! Platform layer: everything that talks to Android.
//!
//! The modules below are only compiled for Android; the host build of this
//! crate exposes the ABI constants (and their layout tests) so that unit tests
//! can run on a developer machine, while the real engine is validated by the
//! arm64 CI build.

pub mod log;
pub mod ndk;

#[cfg(target_os = "android")]
pub mod audio;
#[cfg(target_os = "android")]
pub mod gl;
#[cfg(target_os = "android")]
pub mod jni;
#[cfg(target_os = "android")]
pub mod media;
#[cfg(target_os = "android")]
pub mod sensors;
