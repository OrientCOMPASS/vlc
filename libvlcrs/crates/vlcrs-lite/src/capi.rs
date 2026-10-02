//! C ABI (`include/vlcrs.h`).
//!
//! The engine is normally driven through JNI (see [`crate::jni_api`]), because
//! attaching the output surface and delivering events both need Java objects.
//! This module exposes the *control* surface for integrators that embed
//! `libvlcrs.so` from C/C++ or through another FFI layer (for example a Flutter
//! plugin that reuses a player instance created on the Kotlin side): every call
//! takes the opaque handle returned by `NativeBridge.nativeCreate`.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::api::{self, Status};
use crate::engine::player::Player;

/// Guard a C entry point: never unwind across the FFI boundary.
fn c_guard<T: Clone>(fallback: T, body: impl FnOnce() -> T) -> T {
    let on_panic = fallback.clone();
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(v) => v,
        Err(_) => {
            crate::verror!("panic inside a C ABI call");
            on_panic
        }
    }
}

fn c_player<T: Clone>(handle: i64, fallback: T, f: impl FnOnce(&Player) -> T) -> T {
    let unknown = fallback.clone();
    c_guard(fallback, || match Player::get(handle) {
        Some(p) => f(&p),
        None => unknown,
    })
}

fn cstr(s: &str) -> *const c_char {
    // callers must not free: the buffer is leaked on purpose, and the set of
    // strings is tiny and bounded
    match CString::new(s) {
        Ok(c) => c.into_raw() as *const c_char,
        Err(_) => b"\0".as_ptr() as *const c_char,
    }
}

/// Library version string.
#[no_mangle]
pub extern "C" fn vlcrs_version() -> *const c_char {
    c_guard(std::ptr::null(), || cstr(crate::VERSION))
}

/// Human readable name of a [`api::PlayerState`] value.
#[no_mangle]
pub unsafe extern "C" fn vlcrs_state_name(state: c_int) -> *const c_char {
    c_guard(std::ptr::null(), || {
        cstr(api::PlayerState::from_i32(state).label())
    })
}

/// Human readable name of a projection mode id.
#[no_mangle]
pub unsafe extern "C" fn vlcrs_mode_name(mode: c_int) -> *const c_char {
    c_guard(std::ptr::null(), || {
        cstr(vlcrs_vr::ProjectionMode::from_i32(mode).label())
    })
}

/// Destroy a player.
#[no_mangle]
pub extern "C" fn vlcrs_destroy(handle: i64) -> c_int {
    c_guard(Status::Failed.as_i32(), || Player::destroy(handle).as_i32())
}

/// Set a URI/path media source.
#[no_mangle]
pub unsafe extern "C" fn vlcrs_set_media_uri(handle: i64, uri: *const c_char) -> c_int {
    c_guard(Status::BadArgument.as_i32(), || {
        if uri.is_null() {
            return Status::BadArgument.as_i32();
        }
        let Ok(s) = unsafe { CStr::from_ptr(uri) }.to_str() else {
            return Status::BadArgument.as_i32();
        };
        c_player(handle, Status::BadHandle.as_i32(), |p| {
            p.set_media_uri(s).as_i32()
        })
    })
}

/// Set a descriptor based media source.
#[no_mangle]
pub extern "C" fn vlcrs_set_media_fd(handle: i64, fd: c_int, offset: i64, length: i64) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_media_fd(fd, offset, length).as_i32()
    })
}

/// Start playback.
#[no_mangle]
pub extern "C" fn vlcrs_play(handle: i64) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| p.play().as_i32())
}

/// Pause playback.
#[no_mangle]
pub extern "C" fn vlcrs_pause(handle: i64) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| p.pause().as_i32())
}

/// Resume playback.
#[no_mangle]
pub extern "C" fn vlcrs_resume(handle: i64) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| p.resume().as_i32())
}

/// Stop playback and unload the media.
#[no_mangle]
pub extern "C" fn vlcrs_stop(handle: i64) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| p.stop().as_i32())
}

/// Seek to an absolute position in milliseconds.
#[no_mangle]
pub extern "C" fn vlcrs_seek_to(handle: i64, position_ms: i64) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.seek_to(position_ms).as_i32()
    })
}

/// Current position in milliseconds, `-1` for an unknown handle.
#[no_mangle]
pub extern "C" fn vlcrs_get_time(handle: i64) -> i64 {
    c_player(handle, -1i64, |p| p.time_ms())
}

/// Duration in milliseconds.
#[no_mangle]
pub extern "C" fn vlcrs_get_length(handle: i64) -> i64 {
    c_player(handle, 0i64, |p| p.length_ms())
}

/// Current [`api::PlayerState`] as an integer.
#[no_mangle]
pub extern "C" fn vlcrs_get_state(handle: i64) -> c_int {
    c_player(handle, 0i32, |p| p.state() as i32)
}

/// Set the volume (`0.0 … 1.0`).
#[no_mangle]
pub extern "C" fn vlcrs_set_volume(handle: i64, volume: f32) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_volume(volume).as_i32()
    })
}

/// Select a projection mode (see [`vlcrs_vr::ProjectionMode`]).
#[no_mangle]
pub extern "C" fn vlcrs_set_projection_mode(handle: i64, mode: c_int) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_projection_mode(mode).as_i32()
    })
}

/// The active projection mode id.
#[no_mangle]
pub extern "C" fn vlcrs_get_projection_mode(handle: i64) -> c_int {
    c_player(handle, 0i32, |p| p.projection_mode().as_i32())
}

/// Select the rendered eye (0 = left, 1 = right).
#[no_mangle]
pub extern "C" fn vlcrs_set_eye(handle: i64, eye: c_int) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_eye(eye).as_i32()
    })
}

/// The active eye (0 = left, 1 = right).
#[no_mangle]
pub extern "C" fn vlcrs_get_eye(handle: i64) -> c_int {
    c_player(handle, 0i32, |p| p.eye().as_i32())
}

/// Invert the eye packing order advertised by the container.
#[no_mangle]
pub extern "C" fn vlcrs_set_swap_eyes(handle: i64, swap: c_int) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_swap_eyes(swap != 0).as_i32()
    })
}

/// Set the vertical field of view in degrees.
#[no_mangle]
pub extern "C" fn vlcrs_set_fov(handle: i64, fov_y: f32) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_fov(fov_y).as_i32()
    })
}

/// Pinch zoom (`factor > 1` zooms in).
#[no_mangle]
pub extern "C" fn vlcrs_zoom(handle: i64, factor: f32) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.zoom(factor).as_i32()
    })
}

/// Single finger drag in pixels.
#[no_mangle]
pub extern "C" fn vlcrs_drag_pixels(handle: i64, dx: f32, dy: f32) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.drag_pixels(dx, dy).as_i32()
    })
}

/// Enable/disable gyroscope look-around.
#[no_mangle]
pub extern "C" fn vlcrs_set_gyro(handle: i64, enabled: c_int, display_rotation: c_int) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        let _ = p.set_display_rotation(display_rotation);
        p.set_gyro(enabled != 0).as_i32()
    })
}

/// Recentre the view ("视角摆正").
#[no_mangle]
pub extern "C" fn vlcrs_recenter(handle: i64) -> c_int {
    c_player(handle, Status::BadHandle.as_i32(), |p| {
        p.recenter().as_i32()
    })
}

/// Fill `out` with the compact HUD floats; returns the number written.
#[no_mangle]
pub unsafe extern "C" fn vlcrs_get_view_info(handle: i64, out: *mut f32, count: c_int) -> c_int {
    if out.is_null() || count <= 0 {
        return 0;
    }
    let n = count as usize;
    c_player(handle, 0i32, |p| {
        let mut buf = vec![0f32; n];
        let written = p.view_info(&mut buf);
        unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), out, written) };
        written as c_int
    })
}

/// Fill `out` with the media info ints (see [`api::media_info_array`]).
#[no_mangle]
pub unsafe extern "C" fn vlcrs_get_media_info(handle: i64, out: *mut c_int, count: c_int) -> c_int {
    if out.is_null() || count <= 0 {
        return 0;
    }
    c_player(handle, 0i32, |p| {
        let values = api::media_info_array(&p.media_info());
        let n = (count as usize).min(values.len());
        unsafe { std::ptr::copy_nonoverlapping(values.as_ptr(), out, n) };
        n as c_int
    })
}

/// Fill `out` with the runtime counters:
/// `[decoded, rendered, dropped, underruns, seeks, fps×100, bufferedMs, events]`.
#[no_mangle]
pub unsafe extern "C" fn vlcrs_get_stats(handle: i64, out: *mut i64, count: c_int) -> c_int {
    if out.is_null() || count <= 0 {
        return 0;
    }
    c_player(handle, 0i32, |p| {
        let s = p.stats();
        let values = [
            s.decoded_frames as i64,
            s.rendered_frames as i64,
            s.dropped_frames as i64,
            s.audio_underruns as i64,
            s.seeks as i64,
            (s.fps * 100.0) as i64,
            s.buffered_ms,
            s.events_posted,
        ];
        let n = (count as usize).min(values.len());
        unsafe { std::ptr::copy_nonoverlapping(values.as_ptr(), out, n) };
        n as c_int
    })
}

/// Copy the formatted HUD text into `buf` (NUL terminated); returns the number
/// of bytes written excluding the terminator, or the required length when
/// `len` is 0.
#[no_mangle]
pub unsafe extern "C" fn vlcrs_hud_text(handle: i64, buf: *mut c_char, len: c_int) -> c_int {
    let text = c_player(handle, String::new(), |p| p.hud().lines().join("\n"));
    let bytes = text.as_bytes();
    if buf.is_null() || len <= 0 {
        return bytes.len() as c_int;
    }
    let n = ((len as usize) - 1).min(bytes.len());
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    n as c_int
}

/// Number of floats in the compact HUD representation.
#[no_mangle]
pub extern "C" fn vlcrs_hud_float_count() -> c_int {
    vlcrs_vr::hud::HUD_FLOAT_COUNT as c_int
}
