//! `libvlcrs` — a lightweight, Rust re-implementation of the libvlc playback
//! core for Android (`arm64-v8a`), with first class spherical/VR video support.
//!
//! # Layout
//!
//! * [`api`] — the engine's public, platform independent API (handles, states,
//!   events, VR controls).  Both the JNI layer and the C ABI are thin wrappers
//!   over it.
//! * [`engine`] — playback pipeline: demux thread, video decode thread, audio
//!   decode + AAudio render thread, master clock, seek/flush machinery.
//! * [`render`] — EGL/GLES renderer: sphere / hemisphere / flat geometries,
//!   eye + layout selection, head tracking, HUD.
//! * [`platform`] — Android FFI (`libmediandk`, `libaaudio`, `libandroid`,
//!   `libEGL`, `libGLESv2`, `liblog`) and JNI bridges.
//! * [`capi`] — `extern "C"` surface for integrators that prefer C over JNI
//!   (see `include/vlcrs.h`).
//!
//! Everything below the `api` module is only compiled for Android; the host
//! build still type checks the ABI constants so the layout unit tests can run
//! on a workstation.
//!
//! # Design notes
//!
//! * No bundled codecs: demuxing and decoding go through the platform
//!   (`AMediaExtractor` + `AMediaCodec`), audio output through AAudio.  That is
//!   what keeps the engine *lightweight* (one `.so`, no FFmpeg, no contribs).
//! * The decoded video never touches system memory: `AMediaCodec` renders into
//!   a `SurfaceTexture` owned by our GL context, and the projection is applied
//!   with a shader that samples the external OES texture.
//! * Switching projection mode / eye is a uniform update on the render thread,
//!   so it happens in place and the playback position is preserved.

#![cfg_attr(target_os = "android", deny(unsafe_op_in_unsafe_fn))]
#![warn(missing_docs)]

pub mod api;
pub mod platform;

pub mod engine;
pub mod render;

#[cfg(target_os = "android")]
pub mod capi;
#[cfg(target_os = "android")]
pub mod jni_api;

/// Library version (kept in sync with the crate version).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Name of the produced shared library (`libvlcrs.so`).
pub const LIB_NAME: &str = "vlcrs";
