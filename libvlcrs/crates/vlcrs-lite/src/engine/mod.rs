//! Engine internals.
//!
//! [`clock`] and [`queue`] are platform independent and unit tested on the host;
//! the rest talks to Android and is compiled for `arm64-v8a` only.

pub(crate) mod clock;
pub(crate) mod queue;

#[cfg(target_os = "android")]
pub(crate) mod audio;
#[cfg(target_os = "android")]
pub(crate) mod demux;
#[cfg(target_os = "android")]
pub(crate) mod format;
#[cfg(target_os = "android")]
pub(crate) mod player;
#[cfg(target_os = "android")]
pub(crate) mod state;

#[cfg(target_os = "android")]
pub(crate) use state::{Inner, Source, EOS_AUDIO, EOS_VIDEO};
