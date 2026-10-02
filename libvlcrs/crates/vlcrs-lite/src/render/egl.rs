//! EGL context / surface management.
//!
//! A 1×1 pbuffer keeps the context current while the application has no
//! surface, which matters because the decoder's `SurfaceTexture` and its OES
//! texture are created *inside* our context: they must survive surface
//! destruction (rotation, backgrounding) so playback can resume in place.

use std::ptr;

use crate::platform::gl::*;
use crate::platform::ndk::ANativeWindow;

pub(crate) struct Egl {
    display: EGLDisplay,
    config: EGLConfig,
    context: EGLContext,
    window: EGLSurface,
    pbuffer: EGLSurface,
    pub width: i32,
    pub height: i32,
    pub has_window: bool,
}

impl Egl {
    /// Create the display, config, context and a 1×1 pbuffer, then make the
    /// context current.
    pub fn init() -> Result<Egl, String> {
        unsafe {
            let display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
            if display == EGL_NO_DISPLAY {
                return Err(format!("eglGetDisplay failed: {}", err_hex(eglGetError())));
            }
            let mut major = 0;
            let mut minor = 0;
            if eglInitialize(display, &mut major, &mut minor) == EGL_FALSE {
                return Err(format!("eglInitialize failed: {}", err_hex(eglGetError())));
            }
            let attribs: [EGLint; 15] = [
                EGL_SURFACE_TYPE,
                EGL_WINDOW_BIT | EGL_PBUFFER_BIT,
                EGL_RENDERABLE_TYPE,
                EGL_OPENGL_ES2_BIT,
                EGL_RED_SIZE,
                8,
                EGL_GREEN_SIZE,
                8,
                EGL_BLUE_SIZE,
                8,
                EGL_ALPHA_SIZE,
                8,
                EGL_DEPTH_SIZE,
                0,
                EGL_NONE,
            ];
            let mut config: EGLConfig = ptr::null_mut();
            let mut num = 0;
            if eglChooseConfig(display, attribs.as_ptr(), &mut config, 1, &mut num) == EGL_FALSE
                || num < 1
                || config.is_null()
            {
                return Err(format!(
                    "eglChooseConfig failed: {}",
                    err_hex(eglGetError())
                ));
            }
            let ctx_attribs: [EGLint; 3] = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
            let context = eglCreateContext(display, config, EGL_NO_CONTEXT, ctx_attribs.as_ptr());
            if context == EGL_NO_CONTEXT {
                return Err(format!(
                    "eglCreateContext failed: {}",
                    err_hex(eglGetError())
                ));
            }
            let pb_attribs: [EGLint; 5] = [EGL_PBUFFER_WIDTH, 1, EGL_PBUFFER_HEIGHT, 1, EGL_NONE];
            let pbuffer = eglCreatePbufferSurface(display, config, pb_attribs.as_ptr());
            if pbuffer == EGL_NO_SURFACE {
                eglDestroyContext(display, context);
                return Err(format!(
                    "eglCreatePbufferSurface failed: {}",
                    err_hex(eglGetError())
                ));
            }
            if eglMakeCurrent(display, pbuffer, pbuffer, context) == EGL_FALSE {
                eglDestroySurface(display, pbuffer);
                eglDestroyContext(display, context);
                return Err(format!("eglMakeCurrent failed: {}", err_hex(eglGetError())));
            }
            eglSwapInterval(display, 1);
            crate::vlog!("EGL initialised (v{major}.{minor})");
            Ok(Egl {
                display,
                config,
                context,
                window: EGL_NO_SURFACE,
                pbuffer,
                width: 1,
                height: 1,
                has_window: false,
            })
        }
    }

    /// Attach an application window (creates the window surface).
    pub fn set_window(&mut self, win: *mut ANativeWindow) -> Result<(), String> {
        if win.is_null() {
            return self.clear_window();
        }
        unsafe {
            // drop a previous window surface first
            if self.window != EGL_NO_SURFACE {
                eglMakeCurrent(self.display, self.pbuffer, self.pbuffer, self.context);
                eglDestroySurface(self.display, self.window);
                self.window = EGL_NO_SURFACE;
            }
            let surface = eglCreateWindowSurface(self.display, self.config, win, ptr::null());
            if surface == EGL_NO_SURFACE {
                return Err(format!(
                    "eglCreateWindowSurface failed: {}",
                    err_hex(eglGetError())
                ));
            }
            if eglMakeCurrent(self.display, surface, surface, self.context) == EGL_FALSE {
                eglDestroySurface(self.display, surface);
                return Err(format!("eglMakeCurrent failed: {}", err_hex(eglGetError())));
            }
            self.window = surface;
            self.has_window = true;
            self.query_size();
            eglSwapInterval(self.display, 1);
            crate::vlog!("EGL window surface {}x{}", self.width, self.height);
            Ok(())
        }
    }

    /// Detach the application window, falling back to the pbuffer.
    pub fn clear_window(&mut self) -> Result<(), String> {
        unsafe {
            if self.window != EGL_NO_SURFACE {
                eglMakeCurrent(self.display, self.pbuffer, self.pbuffer, self.context);
                eglDestroySurface(self.display, self.window);
                self.window = EGL_NO_SURFACE;
            }
            self.has_window = false;
            self.width = 1;
            self.height = 1;
            Ok(())
        }
    }

    fn query_size(&mut self) {
        unsafe {
            let surface = self.current_surface();
            let mut w = 0;
            let mut h = 0;
            if eglQuerySurface(self.display, surface, EGL_WIDTH, &mut w) == EGL_TRUE
                && eglQuerySurface(self.display, surface, EGL_HEIGHT, &mut h) == EGL_TRUE
                && w > 0
                && h > 0
            {
                self.width = w;
                self.height = h;
            }
        }
    }

    fn current_surface(&self) -> EGLSurface {
        if self.window != EGL_NO_SURFACE {
            self.window
        } else {
            self.pbuffer
        }
    }

    /// Present.
    pub fn swap(&self) -> bool {
        unsafe { eglSwapBuffers(self.display, self.current_surface()) == EGL_TRUE }
    }

    /// `true` when an application window is attached.
    pub fn has_window(&self) -> bool {
        self.has_window
    }
}

impl Drop for Egl {
    fn drop(&mut self) {
        unsafe {
            if self.display != EGL_NO_DISPLAY {
                eglMakeCurrent(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
                if self.window != EGL_NO_SURFACE {
                    eglDestroySurface(self.display, self.window);
                }
                if self.pbuffer != EGL_NO_SURFACE {
                    eglDestroySurface(self.display, self.pbuffer);
                }
                if self.context != EGL_NO_CONTEXT {
                    eglDestroyContext(self.display, self.context);
                }
                eglTerminate(self.display);
            }
        }
        crate::vlog!("EGL terminated");
    }
}
