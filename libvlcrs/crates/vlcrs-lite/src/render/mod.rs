//! Rendering: EGL context, spherical/flat geometry, eye selection and the
//! draw loop that also drives the video decoder output.

#[cfg(target_os = "android")]
pub(crate) mod egl;
#[cfg(target_os = "android")]
pub(crate) mod renderer;
