//! EGL + OpenGL ES 2.0 FFI and a minimal, explicit context wrapper.
//!
//! The declarations follow `<EGL/egl.h>`, `<EGL/eglplatform.h>` and
//! `<GLES2/gl2.h>` from the NDK sysroot.  `EGLNativeWindowType` is
//! `ANativeWindow*` on Android.
//!
//! Only the entry points the renderer needs are declared, and they are linked
//! directly against `libEGL.so` / `libGLESv2.so` (both are part of the NDK
//! sysroot for every supported API level), which avoids a build script and a
//! loader-function dance.

#![allow(missing_docs)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(clippy::upper_case_acronyms)]

use std::os::raw::{c_char, c_void};

use super::ndk::ANativeWindow;

/// `EGLint`
pub type EGLint = i32;
/// `EGLBoolean`
pub type EGLBoolean = u32;
/// `GLenum`
pub type GLenum = u32;
/// `GLuint`
pub type GLuint = u32;
/// `GLint`
pub type GLint = i32;
/// `GLsizei`
pub type GLsizei = i32;
/// `GLfloat`
pub type GLfloat = f32;
/// `GLboolean`
pub type GLboolean = u8;
/// `GLsizeiptr`
pub type GLsizeiptr = isize;
/// `GLbitfield`
pub type GLbitfield = u32;

/// Opaque EGL handle.
pub type EGLDisplay = *mut c_void;
/// Opaque EGL handle.
pub type EGLSurface = *mut c_void;
/// Opaque EGL handle.
pub type EGLContext = *mut c_void;
/// Opaque EGL handle.
pub type EGLConfig = *mut c_void;

/// `EGL_FALSE`
pub const EGL_FALSE: EGLBoolean = 0;
/// `EGL_TRUE`
pub const EGL_TRUE: EGLBoolean = 1;
/// `EGL_SUCCESS`
pub const EGL_SUCCESS: EGLint = 0x3000;
/// `EGL_DEFAULT_DISPLAY`
pub const EGL_DEFAULT_DISPLAY: *mut c_void = std::ptr::null_mut();
/// `EGL_NO_DISPLAY`
pub const EGL_NO_DISPLAY: EGLDisplay = std::ptr::null_mut();
/// `EGL_NO_SURFACE`
pub const EGL_NO_SURFACE: EGLSurface = std::ptr::null_mut();
/// `EGL_NO_CONTEXT`
pub const EGL_NO_CONTEXT: EGLContext = std::ptr::null_mut();

/// Config attribute: surface types.
pub const EGL_SURFACE_TYPE: EGLint = 0x3033;
/// Config attribute value: window surfaces.
pub const EGL_WINDOW_BIT: EGLint = 0x0004;
/// Config attribute value: pbuffer surfaces.
pub const EGL_PBUFFER_BIT: EGLint = 0x0001;
/// Config attribute: renderable client APIs.
pub const EGL_RENDERABLE_TYPE: EGLint = 0x3040;
/// Config attribute value: OpenGL ES 2.
pub const EGL_OPENGL_ES2_BIT: EGLint = 0x0004;
/// Config attribute: red size.
pub const EGL_RED_SIZE: EGLint = 0x3024;
/// Config attribute: green size.
pub const EGL_GREEN_SIZE: EGLint = 0x3023;
/// Config attribute: blue size.
pub const EGL_BLUE_SIZE: EGLint = 0x3022;
/// Config attribute: alpha size.
pub const EGL_ALPHA_SIZE: EGLint = 0x3021;
/// Config attribute: depth size.
pub const EGL_DEPTH_SIZE: EGLint = 0x3025;
/// Config attribute: stencil size.
pub const EGL_STENCIL_SIZE: EGLint = 0x3026;
/// Config attribute terminator.
pub const EGL_NONE: EGLint = 0x3038;
/// Context attribute: client version.
pub const EGL_CONTEXT_CLIENT_VERSION: EGLint = 0x3098;
/// Surface attribute: width.
pub const EGL_WIDTH: EGLint = 0x3057;
/// Surface attribute: height.
pub const EGL_HEIGHT: EGLint = 0x3058;
/// Pbuffer attribute: width.
pub const EGL_PBUFFER_WIDTH: EGLint = 0x3057;
/// Pbuffer attribute: height.
pub const EGL_PBUFFER_HEIGHT: EGLint = 0x3058;

#[link(name = "EGL")]
extern "C" {
    pub fn eglGetDisplay(display_id: *mut c_void) -> EGLDisplay;
    pub fn eglInitialize(dpy: EGLDisplay, major: *mut EGLint, minor: *mut EGLint) -> EGLBoolean;
    pub fn eglTerminate(dpy: EGLDisplay) -> EGLBoolean;
    pub fn eglGetError() -> EGLint;
    pub fn eglChooseConfig(
        dpy: EGLDisplay,
        attrib_list: *const EGLint,
        configs: *mut EGLConfig,
        config_size: EGLint,
        num_config: *mut EGLint,
    ) -> EGLBoolean;
    pub fn eglCreateWindowSurface(
        dpy: EGLDisplay,
        config: EGLConfig,
        win: *mut ANativeWindow,
        attribs: *const EGLint,
    ) -> EGLSurface;
    pub fn eglCreatePbufferSurface(
        dpy: EGLDisplay,
        config: EGLConfig,
        attribs: *const EGLint,
    ) -> EGLSurface;
    pub fn eglDestroySurface(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
    pub fn eglCreateContext(
        dpy: EGLDisplay,
        config: EGLConfig,
        share_context: EGLContext,
        attribs: *const EGLint,
    ) -> EGLContext;
    pub fn eglDestroyContext(dpy: EGLDisplay, ctx: EGLContext) -> EGLBoolean;
    pub fn eglMakeCurrent(
        dpy: EGLDisplay,
        draw: EGLSurface,
        read: EGLSurface,
        ctx: EGLContext,
    ) -> EGLBoolean;
    pub fn eglSwapBuffers(dpy: EGLDisplay, surface: EGLSurface) -> EGLBoolean;
    pub fn eglSwapInterval(dpy: EGLDisplay, interval: EGLint) -> EGLBoolean;
    pub fn eglQuerySurface(
        dpy: EGLDisplay,
        surface: EGLSurface,
        attribute: EGLint,
        value: *mut EGLint,
    ) -> EGLBoolean;
    pub fn eglGetProcAddress(procname: *const c_char) -> *mut c_void;
}

// ---------------------------------------------------------------------------
// OpenGL ES 2.0
// ---------------------------------------------------------------------------

/// `GL_NO_ERROR`
pub const GL_NO_ERROR: GLenum = 0;
/// `GL_FALSE`
pub const GL_FALSE: GLint = 0;
/// `GL_TRUE`
pub const GL_TRUE: GLint = 1;
/// `GL_TRIANGLES`
pub const GL_TRIANGLES: GLenum = 0x0004;
/// `GL_TRIANGLE_STRIP`
pub const GL_TRIANGLE_STRIP: GLenum = 0x0005;
/// `GL_UNSIGNED_SHORT`
pub const GL_UNSIGNED_SHORT: GLenum = 0x1403;
/// `GL_UNSIGNED_INT`
pub const GL_UNSIGNED_INT: GLenum = 0x1405;
/// `GL_FLOAT`
pub const GL_FLOAT: GLenum = 0x1406;
/// `GL_ARRAY_BUFFER`
pub const GL_ARRAY_BUFFER: GLenum = 0x8892;
/// `GL_ELEMENT_ARRAY_BUFFER`
pub const GL_ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
/// `GL_STATIC_DRAW`
pub const GL_STATIC_DRAW: GLenum = 0x88E4;
/// `GL_DYNAMIC_DRAW`
pub const GL_DYNAMIC_DRAW: GLenum = 0x88E8;
/// `GL_TEXTURE_2D`
pub const GL_TEXTURE_2D: GLenum = 0x0DE1;
/// `GL_TEXTURE_EXTERNAL_OES` (from `GLES2/gl2ext.h`)
pub const GL_TEXTURE_EXTERNAL_OES: GLenum = 0x8D65;
/// `GL_TEXTURE0`
pub const GL_TEXTURE0: GLenum = 0x84C0;
/// `GL_TEXTURE_MIN_FILTER`
pub const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
/// `GL_TEXTURE_MAG_FILTER`
pub const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
/// `GL_TEXTURE_WRAP_S`
pub const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
/// `GL_TEXTURE_WRAP_T`
pub const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
/// `GL_LINEAR`
pub const GL_LINEAR: GLenum = 0x2601;
/// `GL_NEAREST`
pub const GL_NEAREST: GLenum = 0x2600;
/// `GL_CLAMP_TO_EDGE`
pub const GL_CLAMP_TO_EDGE: GLenum = 0x812F;
/// `GL_COLOR_BUFFER_BIT`
pub const GL_COLOR_BUFFER_BIT: GLbitfield = 0x0000_4000;
/// `GL_DEPTH_BUFFER_BIT`
pub const GL_DEPTH_BUFFER_BIT: GLbitfield = 0x0000_0100;
/// `GL_VERTEX_SHADER`
pub const GL_VERTEX_SHADER: GLenum = 0x8B31;
/// `GL_FRAGMENT_SHADER`
pub const GL_FRAGMENT_SHADER: GLenum = 0x8B30;
/// `GL_COMPILE_STATUS`
pub const GL_COMPILE_STATUS: GLenum = 0x8B81;
/// `GL_LINK_STATUS`
pub const GL_LINK_STATUS: GLenum = 0x8B82;
/// `GL_INFO_LOG_LENGTH`
pub const GL_INFO_LOG_LENGTH: GLenum = 0x8B84;
/// `GL_CULL_FACE`
pub const GL_CULL_FACE: GLenum = 0x0B44;
/// `GL_DEPTH_TEST`
pub const GL_DEPTH_TEST: GLenum = 0x0B71;
/// `GL_BLEND`
pub const GL_BLEND: GLenum = 0x0BE2;
/// `GL_BACK`
pub const GL_BACK: GLenum = 0x0405;
/// `GL_CCW`
pub const GL_CCW: GLenum = 0x0901;
/// `GL_VENDOR`
pub const GL_VENDOR: GLenum = 0x1F00;
/// `GL_RENDERER`
pub const GL_RENDERER: GLenum = 0x1F01;
/// `GL_VERSION`
pub const GL_VERSION: GLenum = 0x1F02;
/// `GL_EXTENSIONS`
pub const GL_EXTENSIONS: GLenum = 0x1F03;

#[link(name = "GLESv2")]
extern "C" {
    pub fn glViewport(x: GLint, y: GLint, width: GLsizei, height: GLsizei);
    pub fn glClear(mask: GLbitfield);
    pub fn glClearColor(red: GLfloat, green: GLfloat, blue: GLfloat, alpha: GLfloat);
    pub fn glEnable(cap: GLenum);
    pub fn glDisable(cap: GLenum);
    pub fn glGetError() -> GLenum;
    pub fn glGetString(name: GLenum) -> *const u8;
    pub fn glFlush();
    pub fn glFinish();

    pub fn glCreateShader(shader_type: GLenum) -> GLuint;
    pub fn glShaderSource(
        shader: GLuint,
        count: GLsizei,
        string: *const *const c_char,
        length: *const GLint,
    );
    pub fn glCompileShader(shader: GLuint);
    pub fn glGetShaderiv(shader: GLuint, pname: GLenum, params: *mut GLint);
    pub fn glGetShaderInfoLog(
        shader: GLuint,
        buf_size: GLsizei,
        length: *mut GLsizei,
        info_log: *mut c_char,
    );
    pub fn glDeleteShader(shader: GLuint);

    pub fn glCreateProgram() -> GLuint;
    pub fn glAttachShader(program: GLuint, shader: GLuint);
    pub fn glLinkProgram(program: GLuint);
    pub fn glGetProgramiv(program: GLuint, pname: GLenum, params: *mut GLint);
    pub fn glGetProgramInfoLog(
        program: GLuint,
        buf_size: GLsizei,
        length: *mut GLsizei,
        info_log: *mut c_char,
    );
    pub fn glUseProgram(program: GLuint);
    pub fn glDeleteProgram(program: GLuint);
    pub fn glGetAttribLocation(program: GLuint, name: *const c_char) -> GLint;
    pub fn glGetUniformLocation(program: GLuint, name: *const c_char) -> GLint;

    pub fn glGenBuffers(n: GLsizei, buffers: *mut GLuint);
    pub fn glDeleteBuffers(n: GLsizei, buffers: *const GLuint);
    pub fn glBindBuffer(target: GLenum, buffer: GLuint);
    pub fn glBufferData(target: GLenum, size: GLsizeiptr, data: *const c_void, usage: GLenum);
    pub fn glBufferSubData(
        target: GLenum,
        offset: GLsizeiptr,
        size: GLsizeiptr,
        data: *const c_void,
    );

    pub fn glGenTextures(n: GLsizei, textures: *mut GLuint);
    pub fn glDeleteTextures(n: GLsizei, textures: *const GLuint);
    pub fn glBindTexture(target: GLenum, texture: GLuint);
    pub fn glActiveTexture(texture: GLenum);
    pub fn glTexParameteri(target: GLenum, pname: GLenum, param: GLint);
    pub fn glTexParameterf(target: GLenum, pname: GLenum, param: GLfloat);
    pub fn glTexImage2D(
        target: GLenum,
        level: GLint,
        internalformat: GLint,
        width: GLsizei,
        height: GLsizei,
        border: GLint,
        format: GLenum,
        type_: GLenum,
        pixels: *const c_void,
    );
    pub fn glPixelStorei(pname: GLenum, param: GLint);

    pub fn glVertexAttribPointer(
        index: GLuint,
        size: GLint,
        type_: GLenum,
        normalized: GLboolean,
        stride: GLsizei,
        pointer: *const c_void,
    );
    pub fn glEnableVertexAttribArray(index: GLuint);
    pub fn glDisableVertexAttribArray(index: GLuint);
    pub fn glDrawElements(mode: GLenum, count: GLsizei, type_: GLenum, indices: *const c_void);

    pub fn glUniform1i(location: GLint, v0: GLint);
    pub fn glUniform1f(location: GLint, v0: GLfloat);
    pub fn glUniform2f(location: GLint, v0: GLfloat, v1: GLfloat);
    pub fn glUniform4f(location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat, v3: GLfloat);
    pub fn glUniformMatrix4fv(
        location: GLint,
        count: GLsizei,
        transpose: GLboolean,
        value: *const GLfloat,
    );
}

/// Hex formatting helper for EGL/GL error codes.
pub fn err_hex(code: i32) -> String {
    format!("0x{code:04x}")
}

/// Read a `glGetString` value as a Rust string (empty when unavailable).
///
/// # Safety
/// Must be called with a current GL context on the calling thread.
pub unsafe fn gl_string(name: GLenum) -> String {
    let p = unsafe { glGetString(name) };
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while unsafe { *p.add(len) } != 0 && len < 65536 {
        len += 1;
    }
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(p, len) }).into_owned()
}
