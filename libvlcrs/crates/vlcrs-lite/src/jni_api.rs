//! JNI entry points.
//!
//! The Kotlin side declares these as `external` members of
//! `org.videolan.libvlcrs.NativeBridge`; every call takes the opaque player
//! handle as its first argument.  All entry points are panic-guarded: a Rust
//! panic must never unwind across the JNI boundary.

use std::os::raw::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, OnceLock};

use jni::objects::{JFloatArray, JIntArray, JLongArray, JObject, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, JNI_FALSE, JNI_TRUE, JNI_VERSION_1_6};
use jni::{JNIEnv, JavaVM};

use crate::api::{MediaInfoSnapshot, Options, Status};
use crate::engine::player::Player;
use crate::platform::jni::JavaBridge;

static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

/// `JNI_OnLoad`: capture the VM so engine threads can attach themselves.
#[no_mangle]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _reserved: *mut c_void) -> jint {
    let _ = JAVA_VM.set(vm);
    crate::platform::log::log(
        crate::platform::log::level::INFO,
        &format!("libvlcrs {} loaded", crate::VERSION),
    );
    JNI_VERSION_1_6
}

/// The captured VM, if `JNI_OnLoad` ran.
pub fn java_vm() -> Option<&'static JavaVM> {
    JAVA_VM.get()
}

/// Run a JNI body, converting panics into `fallback`.
fn guard<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(v) => v,
        Err(_) => {
            crate::verror!("panic inside a JNI call");
            fallback
        }
    }
}

/// Resolve a handle and run `f`, returning `fallback` when unknown.
fn with_player<T: Clone>(handle: jlong, fallback: T, f: impl FnOnce(&Player) -> T) -> T {
    let unknown = fallback.clone();
    guard(fallback, || match Player::get(handle) {
        Some(p) => f(&p),
        None => {
            crate::vwarn!("unknown player handle {handle}");
            unknown
        }
    })
}

// ---------------------------------------------------------------------------
// lifecycle
// ---------------------------------------------------------------------------

/// `nativeCreate(callback, surfaceTextureBridge, packageName): Long`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeCreate(
    mut env: JNIEnv,
    _this: JObject,
    callback: JObject,
    surface_bridge: JObject,
    package: JString,
) -> jlong {
    guard(0i64, || {
        if callback.is_null() || surface_bridge.is_null() {
            crate::verror!("nativeCreate: null callback or bridge");
            return 0;
        }
        let Some(vm) = java_vm() else {
            crate::verror!("nativeCreate before JNI_OnLoad");
            return 0;
        };
        let Ok(vm) = (unsafe { JavaVM::from_raw(vm.get_java_vm_pointer()) }) else {
            crate::verror!("nativeCreate: cannot clone the JavaVM");
            return 0;
        };
        let Ok(cb) = env.new_global_ref(callback) else {
            return 0;
        };
        let Ok(stb) = env.new_global_ref(surface_bridge) else {
            return 0;
        };
        let pkg = env
            .get_string(&package)
            .map(String::from)
            .unwrap_or_else(|_| "org.videolan.libvlcrs".to_string());
        let bridge = Arc::new(JavaBridge::new(vm, cb, stb));
        Player::create(bridge, pkg, Options::default())
    })
}

/// `nativeDestroy(handle)`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeDestroy(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) {
    let _ = guard(Status::BadHandle.as_i32(), || {
        Player::destroy(handle).as_i32()
    });
}

// ---------------------------------------------------------------------------
// media / transport
// ---------------------------------------------------------------------------

/// `nativeSetMediaUri(handle, uri): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSetMediaUri(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
    uri: JString,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        let Ok(js) = env.get_string(&uri) else {
            return Status::BadArgument.as_i32();
        };
        let uri = String::from(js);
        p.set_media_uri(&uri).as_i32()
    })
}

/// `nativeSetMediaFd(handle, fd, offset, length): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSetMediaFd(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    fd: jint,
    offset: jlong,
    length: jlong,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_media_fd(fd, offset, length).as_i32()
    })
}

/// `nativePlay(handle): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativePlay(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| p.play().as_i32())
}

/// `nativePause(handle): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativePause(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| p.pause().as_i32())
}

/// `nativeResume(handle): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeResume(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| p.resume().as_i32())
}

/// `nativeStop(handle): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeStop(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| p.stop().as_i32())
}

/// `nativeSeekTo(handle, positionMs): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSeekTo(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    position_ms: jlong,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.seek_to(position_ms).as_i32()
    })
}

/// `nativeGetTime(handle): Long`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeGetTime(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jlong {
    with_player(handle, -1i64, |p| p.time_ms())
}

/// `nativeGetLength(handle): Long`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeGetLength(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jlong {
    with_player(handle, 0i64, |p| p.length_ms())
}

/// `nativeGetState(handle): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeGetState(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jint {
    with_player(handle, 0i32, |p| p.state() as i32)
}

/// `nativeSetVolume(handle, volume): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSetVolume(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    volume: jfloat,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_volume(volume).as_i32()
    })
}

// ---------------------------------------------------------------------------
// surface
// ---------------------------------------------------------------------------

/// `nativeSetSurface(handle, surface?): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSetSurface(
    env: JNIEnv,
    _this: JObject,
    handle: jlong,
    surface: JObject,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        let gref = if surface.is_null() {
            None
        } else {
            env.new_global_ref(surface).ok()
        };
        p.set_surface(gref).as_i32()
    })
}

/// `nativeSurfaceChanged(handle, width, height): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSurfaceChanged(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    width: jint,
    height: jint,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.surface_changed(width, height).as_i32()
    })
}

// ---------------------------------------------------------------------------
// VR controls
// ---------------------------------------------------------------------------

/// `nativeSetProjectionMode(handle, mode): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSetProjectionMode(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    mode: jint,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_projection_mode(mode).as_i32()
    })
}

/// `nativeGetProjectionMode(handle): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeGetProjectionMode(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jint {
    with_player(handle, 0i32, |p| p.projection_mode().as_i32())
}

/// `nativeSetEye(handle, eye): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSetEye(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    eye: jint,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_eye(eye).as_i32()
    })
}

/// `nativeGetEye(handle): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeGetEye(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jint {
    with_player(handle, 0i32, |p| p.eye().as_i32())
}

/// `nativeSetSwapEyes(handle, swap): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSetSwapEyes(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    swap: jboolean,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_swap_eyes(swap == JNI_TRUE).as_i32()
    })
}

/// `nativeSetFov(handle, fovDegrees): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSetFov(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    fov: jfloat,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.set_fov(fov).as_i32()
    })
}

/// `nativeZoom(handle, factor): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeZoom(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    factor: jfloat,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.zoom(factor).as_i32()
    })
}

/// `nativeDrag(handle, dxPixels, dyPixels): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeDrag(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    dx: jfloat,
    dy: jfloat,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.drag_pixels(dx, dy).as_i32()
    })
}

/// `nativeDragDegrees(handle, dYaw, dPitch): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeDragDegrees(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    dyaw: jfloat,
    dpitch: jfloat,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.drag_degrees(dyaw, dpitch).as_i32()
    })
}

/// `nativeSetGyro(handle, enabled, displayRotation): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeSetGyro(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    enabled: jboolean,
    display_rotation: jint,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        let _ = p.set_display_rotation(display_rotation);
        p.set_gyro(enabled == JNI_TRUE).as_i32()
    })
}

/// `nativeRecenter(handle): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeRecenter(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jint {
    with_player(handle, Status::BadHandle.as_i32(), |p| {
        p.recenter().as_i32()
    })
}

/// `nativeIsGyroEnabled(handle): Boolean`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeIsGyroEnabled(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jboolean {
    with_player(handle, JNI_FALSE, |p| {
        if p.inner().gyro.load(std::sync::atomic::Ordering::Relaxed) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    })
}

// ---------------------------------------------------------------------------
// read-outs
// ---------------------------------------------------------------------------

/// `nativeGetViewInfo(handle, out: FloatArray): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeGetViewInfo(
    env: JNIEnv,
    _this: JObject,
    handle: jlong,
    out: JFloatArray,
) -> jint {
    with_player(handle, 0i32, |p| {
        let len = match env.get_array_length(&out) {
            Ok(l) => l as usize,
            Err(_) => return 0,
        };
        let mut buf = vec![0f32; len];
        let n = p.view_info(&mut buf);
        if n > 0 && env.set_float_array_region(&out, 0, &buf[..n]).is_err() {
            return 0;
        }
        n as jint
    })
}

/// `nativeGetMediaInfo(handle, out: IntArray): Int`
///
/// Layout: `[width, height, rotation, hasSpherical, coverage, layout, container,
/// sampleRate, channels, fps×100]`.
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeGetMediaInfo(
    env: JNIEnv,
    _this: JObject,
    handle: jlong,
    out: JIntArray,
) -> jint {
    with_player(handle, 0i32, |p| {
        let info: MediaInfoSnapshot = p.media_info();
        let values = media_info_array(&info);
        let len = match env.get_array_length(&out) {
            Ok(l) => (l as usize).min(values.len()),
            Err(_) => return 0,
        };
        if env.set_int_array_region(&out, 0, &values[..len]).is_err() {
            return 0;
        }
        len as jint
    })
}

/// `nativeGetStats(handle, out: LongArray): Int`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeGetStats(
    env: JNIEnv,
    _this: JObject,
    handle: jlong,
    out: JLongArray,
) -> jint {
    with_player(handle, 0i32, |p| {
        let s = p.stats();
        let values = [
            s.decoded_frames as jlong,
            s.rendered_frames as jlong,
            s.dropped_frames as jlong,
            s.audio_underruns as jlong,
            s.seeks as jlong,
            (s.fps * 100.0) as jlong,
            s.buffered_ms as jlong,
            s.events_posted as jlong,
        ];
        let len = match env.get_array_length(&out) {
            Ok(l) => (l as usize).min(values.len()),
            Err(_) => return 0,
        };
        if env.set_long_array_region(&out, 0, &values[..len]).is_err() {
            return 0;
        }
        len as jint
    })
}

/// `nativeGetHudText(handle): String` — the formatted HUD lines, `\n` separated.
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeGetHudText<'local>(
    env: JNIEnv<'local>,
    _this: JObject,
    handle: jlong,
) -> JString<'local> {
    let text = with_player(handle, String::new(), |p| p.hud().lines().join("\n"));
    match env.new_string(text) {
        Ok(s) => s,
        Err(_) => JString::default(),
    }
}

/// `nativeDescribeProjection(handle): String`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeDescribeProjection<
    'local,
>(
    env: JNIEnv<'local>,
    _this: JObject,
    handle: jlong,
) -> JString<'local> {
    let text = with_player(handle, String::new(), |p| {
        p.resolved_projection().describe()
    });
    match env.new_string(text) {
        Ok(s) => s,
        Err(_) => JString::default(),
    }
}

/// `nativeVersion(): String`
#[no_mangle]
pub unsafe extern "system" fn Java_org_videolan_libvlcrs_NativeBridge_nativeVersion<'local>(
    env: JNIEnv<'local>,
    _this: JObject,
) -> JString<'local> {
    match env.new_string(crate::VERSION) {
        Ok(s) => s,
        Err(_) => JString::default(),
    }
}

/// Convert the media snapshot into the JNI int array layout.
pub fn media_info_array(info: &MediaInfoSnapshot) -> [jint; 10] {
    crate::api::media_info_array(info)
}
