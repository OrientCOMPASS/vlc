//! JNI bridges: the event callback into the application and the
//! `SurfaceTexture` helper that lets the *native* GL context own the decoder's
//! output surface.
//!
//! Both Java objects are captured as [`GlobalRef`]s when the engine is created
//! (on a JNI thread), so no `FindClass` is ever needed from an engine thread —
//! `FindClass` from an attached native thread resolves against the system
//! class loader and cannot see application classes.

use std::os::raw::c_void;
use std::sync::atomic::{AtomicI64, Ordering};

use jni::objects::{GlobalRef, JValue};
use jni::sys::{jfloat, jint, jlong};
use jni::{JNIEnv, JavaVM};

use super::ndk::{ANativeWindow, ANativeWindow_fromSurface, ANativeWindow_release};

/// Signature of the Kotlin event callback:
/// `fun onNativeEvent(handle: Long, type: Int, arg1: Int, arg2: Int)`.
pub const CALLBACK_METHOD: &str = "onNativeEvent";
/// Signature of the Kotlin event callback.
pub const CALLBACK_SIG: &str = "(JIII)V";

/// Bridges to the Java side.
pub struct JavaBridge {
    vm: JavaVM,
    callback: GlobalRef,
    surface_bridge: GlobalRef,
    posted: AtomicI64,
}

// `JavaVM` and `GlobalRef` are both thread safe in practice; the jni crate
// marks them so, but our struct also holds the raw pointers used below.
unsafe impl Send for JavaBridge {}
unsafe impl Sync for JavaBridge {}

/// An `ANativeWindow` reference obtained from a Java `Surface`.
pub struct NativeWindow {
    ptr: *mut ANativeWindow,
}

impl NativeWindow {
    /// Raw pointer for `AMediaCodec_configure` / `eglCreateWindowSurface`.
    pub fn as_ptr(&self) -> *mut ANativeWindow {
        self.ptr
    }

    /// Width of the window's buffers, `0` when unknown.
    pub fn width(&self) -> i32 {
        unsafe { super::ndk::ANativeWindow_getWidth(self.ptr) }
    }

    /// Height of the window's buffers, `0` when unknown.
    pub fn height(&self) -> i32 {
        unsafe { super::ndk::ANativeWindow_getHeight(self.ptr) }
    }
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { ANativeWindow_release(self.ptr) };
        }
    }
}

// ANativeWindow references are refcounted by the platform and can be handed
// between the render thread and the video decode thread.
unsafe impl Send for NativeWindow {}

impl JavaBridge {
    /// Create the bridge from objects captured on a JNI thread.
    pub fn new(vm: JavaVM, callback: GlobalRef, surface_bridge: GlobalRef) -> JavaBridge {
        JavaBridge {
            vm,
            callback,
            surface_bridge,
            posted: AtomicI64::new(0),
        }
    }

    /// The Java VM, for threads that need to attach themselves.
    pub fn vm(&self) -> &JavaVM {
        &self.vm
    }

    /// Attach the current thread permanently (cheap if already attached).
    pub fn attach_permanently(&self) -> Option<JNIEnv<'_>> {
        match self.vm.attach_current_thread_permanently() {
            Ok(env) => Some(env),
            Err(e) => {
                crate::verror!("attach_current_thread_permanently failed: {e}");
                None
            }
        }
    }

    /// Number of events posted so far (diagnostics).
    pub fn posted(&self) -> i64 {
        self.posted.load(Ordering::Relaxed)
    }

    /// Deliver an event to the application.
    pub fn post_event(&self, handle: jlong, kind: jint, arg1: jint, arg2: jint) {
        let Ok(mut env) = self.vm.attach_current_thread() else {
            return;
        };
        let args = [
            JValue::Long(handle),
            JValue::Int(kind),
            JValue::Int(arg1),
            JValue::Int(arg2),
        ];
        let r = env.call_method(self.callback.as_obj(), CALLBACK_METHOD, CALLBACK_SIG, &args);
        if let Err(e) = r {
            crate::vwarn!("post_event({kind}) failed: {e}");
        }
        clear_exception(&mut env);
        self.posted.fetch_add(1, Ordering::Relaxed);
    }

    /// Create a `SurfaceTexture` bound to `tex_name` **on the calling thread**
    /// (which must have our EGL context current) and return its `Surface`.
    pub fn create_decoder_surface(&self, tex_name: u32) -> Option<GlobalRef> {
        let mut env = self.vm.attach_current_thread().ok()?;
        let v = env
            .call_method(
                self.surface_bridge.as_obj(),
                "createSurfaceTexture",
                "(I)Landroid/view/Surface;",
                &[JValue::Int(tex_name as jint)],
            )
            .ok()?;
        clear_exception(&mut env);
        let Ok(obj) = v.l() else {
            crate::verror!("createSurfaceTexture did not return an object");
            return None;
        };
        if obj.is_null() {
            crate::verror!("createSurfaceTexture returned null");
            return None;
        }
        env.new_global_ref(obj).ok()
    }

    /// `SurfaceTexture.updateTexImage()` — must be called with our GL context
    /// current on the calling thread.
    pub fn update_tex_image(&self) -> bool {
        let Ok(mut env) = self.vm.attach_current_thread() else {
            return false;
        };
        let r = env.call_method(self.surface_bridge.as_obj(), "updateTexImage", "()V", &[]);
        let ok = r.is_ok();
        if !ok {
            crate::vwarn!("updateTexImage failed: {:?}", r.err());
        }
        clear_exception(&mut env);
        ok
    }

    /// `SurfaceTexture.getTransformMatrix()` → 16 floats (column major).
    pub fn get_transform_matrix(&self, out: &mut [f32; 16]) -> bool {
        let Ok(mut env) = self.vm.attach_current_thread() else {
            return false;
        };
        let v = env.call_method(
            self.surface_bridge.as_obj(),
            "getTransformMatrix",
            "()[F",
            &[],
        );
        let Ok(v) = v else {
            clear_exception(&mut env);
            return false;
        };
        let Ok(obj) = v.l() else {
            clear_exception(&mut env);
            return false;
        };
        if obj.is_null() {
            clear_exception(&mut env);
            return false;
        }
        let array = unsafe { jni::objects::JFloatArray::from_raw(obj.as_raw()) };
        let mut buf = [0f32; 16];
        let r = env.get_float_array_region(array, 0, &mut buf);
        clear_exception(&mut env);
        if r.is_err() {
            return false;
        }
        out.copy_from_slice(&buf);
        true
    }

    /// Release the Java `SurfaceTexture`/`Surface` held by the bridge.
    pub fn release_decoder_surface(&self) {
        let Ok(mut env) = self.vm.attach_current_thread() else {
            return;
        };
        let _ = env.call_method(self.surface_bridge.as_obj(), "release", "()V", &[]);
        clear_exception(&mut env);
    }

    /// Convert a Java `Surface` into an `ANativeWindow` reference.
    pub fn native_window(&self, surface: &GlobalRef) -> Option<NativeWindow> {
        let env = self.vm.attach_current_thread().ok()?;
        let env_ptr = env.get_native_interface() as *mut c_void;
        let ptr = unsafe { ANativeWindow_fromSurface(env_ptr, surface.as_raw() as *mut c_void) };
        if ptr.is_null() {
            crate::verror!("ANativeWindow_fromSurface returned null");
            None
        } else {
            Some(NativeWindow { ptr })
        }
    }

    /// Write the HUD floats into a Java `float[]` held by the caller.
    pub fn fill_float_array(&self, array: &jni::objects::JFloatArray<'_>, values: &[jfloat]) {
        let Ok(env) = self.vm.attach_current_thread() else {
            return;
        };
        let _ = env.set_float_array_region(array, 0, values);
    }
}

/// A Java `Surface` handed to us by the application.
pub struct AppSurface {
    global: GlobalRef,
}

impl AppSurface {
    /// Take ownership of a local `Surface` reference.
    pub fn new(global: GlobalRef) -> AppSurface {
        AppSurface { global }
    }

    /// The underlying global reference.
    pub fn as_ref(&self) -> &GlobalRef {
        &self.global
    }
}

fn clear_exception(env: &mut JNIEnv) {
    if let Ok(true) = env.exception_check() {
        // print the stack trace to logcat, then drop the pending exception
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
}
