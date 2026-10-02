//! The render thread: owns the EGL context, the GL objects, the video decoder
//! and the head tracker sampling.
//!
//! Decoding and rendering live on the *same* thread on purpose: `AMediaCodec`
//! is not `Send`, and keeping the codec next to the code that releases its
//! output buffers removes a whole class of lifetime bugs.  Hardware decoding is
//! asynchronous anyway — the thread only queues compressed packets and collects
//! decoded buffers.

use std::collections::VecDeque;
use std::ffi::CString;
use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vlcrs_vr::hud::{HudSnapshot, HUD_FLOAT_COUNT};
use vlcrs_vr::mesh::{mesh_for, Mesh};
use vlcrs_vr::projection::{Coverage, Geometry, MediaHints, ResolvedProjection, StereoLayout};
use vlcrs_vr::shaders::{attrib, uniform, FS_OES, FS_SOLID, VS_FLAT, VS_RECT, VS_SPHERE};
use vlcrs_vr::view::Viewport;
use vlcrs_vr::{ProjectionSource, UvRect, ViewState};

use crate::engine::format::TrackFormat;
use crate::engine::queue::Packet;
use crate::engine::{Inner, EOS_VIDEO};
use crate::platform::gl::*;
use crate::platform::jni::NativeWindow;
use crate::platform::media::{set_thread_name, BufferInfo, Codec, Dequeue, Format};
use crate::platform::ndk::{key, AMEDIA_OK};
use crate::render::egl::Egl;

/// How long a decoded frame may be late before it is dropped (µs).
const LATE_FRAME_US: i64 = 60_000;
/// Frames due within this window are presented early (µs).
const PRESENT_AHEAD_US: i64 = 8_000;
/// Sleep when there is nothing to do.
const IDLE_SLEEP: Duration = Duration::from_millis(10);
/// Maximum number of decoded frames held before presentation.
const MAX_PENDING: usize = 6;

struct GlMesh {
    pos: GLuint,
    uv: GLuint,
    idx: GLuint,
    count: GLsizei,
}

impl GlMesh {
    fn upload(m: &Mesh) -> Result<GlMesh, String> {
        let mut bufs = [0u32; 3];
        unsafe {
            glGenBuffers(3, bufs.as_mut_ptr());
            if bufs[0] == 0 || bufs[1] == 0 || bufs[2] == 0 {
                return Err("glGenBuffers failed".into());
            }
            glBindBuffer(GL_ARRAY_BUFFER, bufs[0]);
            glBufferData(
                GL_ARRAY_BUFFER,
                (m.positions.len() * 4) as GLsizeiptr,
                m.positions.as_ptr() as *const std::os::raw::c_void,
                GL_STATIC_DRAW,
            );
            glBindBuffer(GL_ARRAY_BUFFER, bufs[1]);
            glBufferData(
                GL_ARRAY_BUFFER,
                (m.texcoords.len() * 4) as GLsizeiptr,
                m.texcoords.as_ptr() as *const std::os::raw::c_void,
                GL_STATIC_DRAW,
            );
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, bufs[2]);
            glBufferData(
                GL_ELEMENT_ARRAY_BUFFER,
                (m.indices.len() * 2) as GLsizeiptr,
                m.indices.as_ptr() as *const std::os::raw::c_void,
                GL_STATIC_DRAW,
            );
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            let err = glGetError();
            if err != GL_NO_ERROR {
                return Err(format!("mesh upload GL error {}", err_hex(err as i32)));
            }
        }
        Ok(GlMesh {
            pos: bufs[0],
            uv: bufs[1],
            idx: bufs[2],
            count: m.index_count() as GLsizei,
        })
    }

    fn delete(&self) {
        unsafe {
            let bufs = [self.pos, self.uv, self.idx];
            glDeleteBuffers(3, bufs.as_ptr());
        }
    }
}

struct Program {
    id: GLuint,
    a_pos: GLint,
    a_uv: GLint,
    u_model: GLint,
    u_view: GLint,
    u_proj: GLint,
    u_uv_rect: GLint,
    u_st: GLint,
    u_tex: GLint,
    u_scale: GLint,
    u_rotation: GLint,
    u_color: GLint,
}

impl Program {
    fn delete(&self) {
        unsafe { glDeleteProgram(self.id) };
    }
}

fn compile(kind: GLenum, src: &str) -> Result<GLuint, String> {
    unsafe {
        let sh = glCreateShader(kind);
        if sh == 0 {
            return Err("glCreateShader failed".into());
        }
        let c = CString::new(src).map_err(|e| e.to_string())?;
        let p = c.as_ptr();
        glShaderSource(sh, 1, &p, ptr::null());
        glCompileShader(sh);
        let mut ok = 0;
        glGetShaderiv(sh, GL_COMPILE_STATUS, &mut ok);
        if ok == GL_FALSE {
            let mut len = 0;
            glGetShaderiv(sh, GL_INFO_LOG_LENGTH, &mut len);
            let mut log: Vec<c_char> = vec![0; len.max(1) as usize + 1];
            glGetShaderInfoLog(sh, len + 1, ptr::null_mut(), log.as_mut_ptr());
            let msg = String::from_utf8_lossy(&log)
                .trim_matches(char::from(0))
                .to_string();
            glDeleteShader(sh);
            return Err(format!("shader compile failed: {msg}"));
        }
        Ok(sh)
    }
}

fn link(vs_src: &str, fs_src: &str) -> Result<Program, String> {
    let vs = compile(GL_VERTEX_SHADER, vs_src)?;
    let fs = compile(GL_FRAGMENT_SHADER, fs_src)?;
    unsafe {
        let prog = glCreateProgram();
        if prog == 0 {
            glDeleteShader(vs);
            glDeleteShader(fs);
            return Err("glCreateProgram failed".into());
        }
        glAttachShader(prog, vs);
        glAttachShader(prog, fs);
        glLinkProgram(prog);
        let mut ok = 0;
        glGetProgramiv(prog, GL_LINK_STATUS, &mut ok);
        if ok == GL_FALSE {
            let mut len = 0;
            glGetProgramiv(prog, GL_INFO_LOG_LENGTH, &mut len);
            let mut log: Vec<c_char> = vec![0; len.max(1) as usize + 1];
            glGetProgramInfoLog(prog, len + 1, ptr::null_mut(), log.as_mut_ptr());
            let msg = String::from_utf8_lossy(&log)
                .trim_matches(char::from(0))
                .to_string();
            glDeleteProgram(prog);
            glDeleteShader(vs);
            glDeleteShader(fs);
            return Err(format!("program link failed: {msg}"));
        }
        glDeleteShader(vs);
        glDeleteShader(fs);
        let loc = |name: &str| -> GLint {
            let c = CString::new(name).unwrap_or_default();
            glGetUniformLocation(prog, c.as_ptr())
        };
        let attrib_loc = |name: &str| -> GLint {
            let c = CString::new(name).unwrap_or_default();
            glGetAttribLocation(prog, c.as_ptr())
        };
        Ok(Program {
            id: prog,
            a_pos: attrib_loc(attrib::POSITION),
            a_uv: attrib_loc(attrib::TEXCOORD),
            u_model: loc(uniform::MODEL),
            u_view: loc(uniform::VIEW),
            u_proj: loc(uniform::PROJECTION),
            u_uv_rect: loc(uniform::UV_RECT),
            u_st: loc(uniform::ST_MATRIX),
            u_tex: loc(uniform::TEXTURE),
            u_scale: loc(uniform::SCALE),
            u_rotation: loc(uniform::ROTATION),
            u_color: loc(uniform::COLOR),
        })
    }
}

/// Video decode + presentation state, owned by the render thread.
struct VideoPath {
    codec: Option<Codec>,
    window: Option<NativeWindow>,
    surface: Option<jni::objects::GlobalRef>,
    pending: VecDeque<(usize, BufferInfo)>,
    output_size: (i32, i32),
    output_rotation: i32,
    input_eos: bool,
    output_eos: bool,
    epoch: u64,
    pending_packet: Option<Packet>,
    configured_mime: String,
}

impl VideoPath {
    fn new() -> VideoPath {
        VideoPath {
            codec: None,
            window: None,
            surface: None,
            pending: VecDeque::new(),
            output_size: (0, 0),
            output_rotation: 0,
            input_eos: false,
            output_eos: false,
            epoch: 0,
            pending_packet: None,
            configured_mime: String::new(),
        }
    }

    fn reset_state(&mut self) {
        self.pending.clear();
        self.input_eos = false;
        self.output_eos = false;
        self.pending_packet = None;
    }
}

pub(crate) struct Renderer {
    egl: Option<Egl>,
    sphere_prog: Option<Program>,
    rect_prog: Option<Program>,
    solid_prog: Option<Program>,
    oes_tex: GLuint,
    tex_ready: bool,
    /// Slot 0: flat quad, slot 1: spherical (360 or 180).
    meshes: [(Option<GlMesh>, Coverage); 2],
    mesh_step: i32,
    video: VideoPath,
    app_window: Option<NativeWindow>,
    st_matrix: [f32; 16],
    had_frame: bool,
    last_fps_sample: Instant,
    frames_since_fps: u32,
    first_frame_posted: bool,
    resolution: Option<ResolvedProjection>,
}

impl Renderer {
    fn new(mesh_step: i32) -> Renderer {
        Renderer {
            egl: None,
            sphere_prog: None,
            rect_prog: None,
            solid_prog: None,
            oes_tex: 0,
            tex_ready: false,
            meshes: [(None, Coverage::Planar), (None, Coverage::Full360)],
            mesh_step: mesh_step.clamp(1, 8),
            video: VideoPath::new(),
            app_window: None,
            st_matrix: [
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                0.0, 0.0, 0.0, 1.0, //
            ],
            had_frame: false,
            last_fps_sample: Instant::now(),
            frames_since_fps: 0,
            first_frame_posted: false,
            resolution: None,
        }
    }

    // ---------------------------------------------------------------- GL init

    fn ensure_gl(&mut self) -> bool {
        if self.egl.is_some() {
            return self.sphere_prog.is_some() && self.rect_prog.is_some();
        }
        match Egl::init() {
            Ok(egl) => self.egl = Some(egl),
            Err(e) => {
                crate::verror!("EGL init failed: {e}");
                return false;
            }
        }
        unsafe {
            glDisable(GL_CULL_FACE);
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_BLEND);
            glClearColor(0.0, 0.0, 0.0, 1.0);
        }
        for (slot, vs, fs) in [
            (&mut self.sphere_prog, VS_SPHERE, FS_OES),
            (&mut self.rect_prog, VS_RECT, FS_OES),
            (&mut self.solid_prog, VS_FLAT, FS_SOLID),
        ] {
            match link(vs, fs) {
                Ok(p) => *slot = Some(p),
                Err(e) => crate::verror!("{e}"),
            }
        }
        self.sphere_prog.is_some() && self.rect_prog.is_some()
    }

    fn ensure_oes_texture(&mut self) -> bool {
        if self.tex_ready {
            return true;
        }
        unsafe {
            let mut tex = 0u32;
            glGenTextures(1, &mut tex);
            if tex == 0 {
                crate::verror!("glGenTextures failed");
                return false;
            }
            glBindTexture(GL_TEXTURE_EXTERNAL_OES, tex);
            glTexParameteri(
                GL_TEXTURE_EXTERNAL_OES,
                GL_TEXTURE_MIN_FILTER,
                GL_LINEAR as GLint,
            );
            glTexParameteri(
                GL_TEXTURE_EXTERNAL_OES,
                GL_TEXTURE_MAG_FILTER,
                GL_LINEAR as GLint,
            );
            glTexParameteri(
                GL_TEXTURE_EXTERNAL_OES,
                GL_TEXTURE_WRAP_S,
                GL_CLAMP_TO_EDGE as GLint,
            );
            glTexParameteri(
                GL_TEXTURE_EXTERNAL_OES,
                GL_TEXTURE_WRAP_T,
                GL_CLAMP_TO_EDGE as GLint,
            );
            glBindTexture(GL_TEXTURE_EXTERNAL_OES, 0);
            self.oes_tex = tex;
        }
        self.tex_ready = true;
        crate::vlog!("created external OES texture {}", self.oes_tex);
        true
    }

    fn ensure_decoder_surface(&mut self, inner: &Arc<Inner>) -> bool {
        if self.video.window.is_some() {
            return true;
        }
        if !self.ensure_oes_texture() {
            return false;
        }
        let Some(surface) = inner.bridge.create_decoder_surface(self.oes_tex) else {
            crate::verror!("could not create the decoder SurfaceTexture");
            return false;
        };
        let Some(window) = inner.bridge.native_window(&surface) else {
            crate::verror!("ANativeWindow_fromSurface failed");
            return false;
        };
        crate::vlog!(
            "decoder surface ready ({}x{})",
            window.width(),
            window.height()
        );
        self.video.surface = Some(surface);
        self.video.window = Some(window);
        true
    }

    fn ensure_codec(&mut self, inner: &Arc<Inner>) -> bool {
        if self.video.codec.is_some() {
            return true;
        }
        if !self.ensure_decoder_surface(inner) {
            return false;
        }
        let tf: Option<TrackFormat> = inner.video_format.lock().ok().and_then(|g| g.clone());
        let Some(tf) = tf else {
            return false;
        };
        let Some(window) = self.video.window.as_ref() else {
            return false;
        };
        let software = inner
            .options
            .lock()
            .map(|o| o.force_software_decoder)
            .unwrap_or(false);
        let mut codec = if software {
            software_decoder(&tf.mime).and_then(|name| {
                crate::vlog!("trying software decoder {name}");
                Codec::create_by_name(&name)
            })
        } else {
            None
        };
        if codec.is_none() {
            codec = Codec::create_decoder(&tf.mime);
        }
        let Some(mut codec) = codec else {
            crate::verror!("no video decoder for {}", tf.mime);
            inner.post(
                crate::api::EventType::Error,
                crate::api::ErrorCode::VideoDecoderFailed as i32,
                0,
            );
            return false;
        };
        let Some(fmt) = tf.build() else {
            crate::verror!("could not rebuild the video format");
            return false;
        };
        let st = codec.configure(&fmt, window.as_ptr(), 0);
        if st != AMEDIA_OK {
            crate::verror!(
                "AMediaCodec_configure failed: {st} ({})",
                crate::platform::ndk::media_status_name(st)
            );
            inner.post(
                crate::api::EventType::Error,
                crate::api::ErrorCode::VideoConfigureFailed as i32,
                st,
            );
            return false;
        }
        let st = codec.start();
        if st != AMEDIA_OK {
            crate::verror!("AMediaCodec_start failed: {st}");
            inner.post(
                crate::api::EventType::Error,
                crate::api::ErrorCode::VideoConfigureFailed as i32,
                st,
            );
            return false;
        }
        crate::vlog!(
            "video decoder started for {} ({})",
            tf.mime,
            codec.name().unwrap_or_else(|| "?".into())
        );
        self.video.codec = Some(codec);
        self.video.configured_mime = tf.mime.clone();
        self.video.reset_state();
        self.video.epoch = inner.current_epoch();
        self.had_frame = false;
        self.first_frame_posted = false;
        true
    }

    fn teardown_codec(&mut self) {
        if let Some(mut c) = self.video.codec.take() {
            let _ = c.stop();
        }
        self.video.reset_state();
        self.video.configured_mime.clear();
        self.had_frame = false;
        self.first_frame_posted = false;
    }

    // ------------------------------------------------------------- video pump

    fn pump_video(&mut self, inner: &Arc<Inner>) {
        let Some(mut codec) = self.video.codec.take() else {
            return;
        };
        self.pump_video_with(inner, &mut codec);
        self.video.codec = Some(codec);
    }

    fn pump_video_with(&mut self, inner: &Arc<Inner>, codec: &mut Codec) {
        let epoch = inner.current_epoch();
        if epoch != self.video.epoch {
            self.video.epoch = epoch;
            let _ = codec.flush();
            self.video.reset_state();
            inner.video_q.drop_stale(epoch);
        }

        // 1. feed the decoder.  An input buffer is only dequeued once a packet
        //    is in hand: a dequeued-but-unqueued buffer would leak from the
        //    codec's pool.
        if !self.video.input_eos {
            for _ in 0..8 {
                if self.video.pending_packet.is_none() {
                    self.video.pending_packet = inner.video_q.pop(Duration::ZERO);
                }
                let Some(pkt) = self.video.pending_packet.take() else {
                    break;
                };
                if pkt.epoch != epoch {
                    continue;
                }
                let Some(idx) = codec.dequeue_input_buffer(0) else {
                    self.video.pending_packet = Some(pkt);
                    break;
                };
                let st = if pkt.eos {
                    codec.queue_eos(idx, pkt.pts_us)
                } else {
                    codec.queue_input(idx, &pkt.data, pkt.pts_us, pkt.flags)
                };
                if st != AMEDIA_OK {
                    crate::vwarn!("queueInputBuffer failed: {st}");
                }
                if pkt.eos {
                    self.video.input_eos = true;
                    break;
                }
            }
        }

        // 2. collect decoded buffers
        for _ in 0..16 {
            match codec.dequeue_output_buffer(0) {
                Dequeue::Buffer(idx, bi) => {
                    if bi.is_codec_config() {
                        let _ = codec.release_output_buffer(idx, false);
                        continue;
                    }
                    if bi.is_eos() {
                        self.video.output_eos = true;
                        let _ = codec.release_output_buffer(idx, false);
                        break;
                    }
                    self.video.pending.push_back((idx, bi));
                    if self.video.pending.len() >= MAX_PENDING {
                        break;
                    }
                }
                Dequeue::FormatChanged => {
                    if let Some(f) = codec.output_format() {
                        self.apply_output_format(inner, &f);
                    }
                }
                Dequeue::TryAgain | Dequeue::BuffersChanged => break,
                Dequeue::Error(e) => {
                    crate::vwarn!("dequeueOutputBuffer error {e}");
                    if Codec::is_recoverable(e) {
                        inner.post(
                            crate::api::EventType::Error,
                            crate::api::ErrorCode::DecodeFailed as i32,
                            e,
                        );
                        self.teardown_codec();
                    }
                    break;
                }
            }
        }
    }

    fn apply_output_format(&mut self, inner: &Arc<Inner>, f: &Format) {
        let w = f.get_i32(key::WIDTH).unwrap_or(0);
        let h = f.get_i32(key::HEIGHT).unwrap_or(0);
        let (w, h) = match (
            f.get_i32(key::CROP_LEFT),
            f.get_i32(key::CROP_TOP),
            f.get_i32(key::CROP_RIGHT),
            f.get_i32(key::CROP_BOTTOM),
        ) {
            (Some(l), Some(t), Some(r), Some(b)) if r > l && b > t => (r - l + 1, b - t + 1),
            _ => (w, h),
        };
        let rot = f.get_i32(key::ROTATION).unwrap_or(0);
        self.video.output_size = (w, h);
        if rot != 0 {
            self.video.output_rotation = rot;
        }
        crate::vlog!("video output format: {}", f.describe());
        if w <= 0 || h <= 0 {
            return;
        }
        // Merge the authoritative decoded size with the probed metadata.
        let spherical = inner
            .spherical
            .lock()
            .ok()
            .and_then(|g| g.as_ref().copied());
        let mut coverage = None;
        let mut rotation = rot;
        if let Ok(mut hints) = inner.hints.lock() {
            hints.width = w as u32;
            hints.height = h as u32;
            if rot == 0 {
                rotation = hints.rotation;
            }
            hints.rotation = rotation;
            if let Some(s) = spherical {
                hints.coverage = s.coverage(hints.width, hints.height);
            }
            coverage = hints.coverage;
        }
        if let Ok(mut info) = inner.info.lock() {
            info.width = w;
            info.height = h;
            info.rotation = rotation;
            info.coverage = crate::api::coverage_id(coverage);
        }
        inner.post(crate::api::EventType::VideoSize, w, h);
        // the projection may depend on the real picture size
        self.resolution = None;
    }

    /// Present the frame that is due (if any) and refresh the OES texture.
    fn present_due_frame(&mut self, inner: &Arc<Inner>) {
        let Some(mut codec) = self.video.codec.take() else {
            return;
        };
        self.present_due_frame_with(inner, &mut codec);
        self.video.codec = Some(codec);
    }

    fn present_due_frame_with(&mut self, inner: &Arc<Inner>, codec: &mut Codec) {
        if self.video.pending.is_empty() {
            if self.video.output_eos {
                inner.mark_eos(EOS_VIDEO);
            }
            return;
        }
        let audio_master = inner.clock.is_audio_master();
        let now = inner.clock.now_us();
        // Drop frames that are hopelessly late, but always keep the newest one.
        while self.video.pending.len() > 1 {
            let late = audio_master && inner.clock.is_valid() && {
                let head_pts = self.video.pending[0].1.pts_us;
                head_pts + LATE_FRAME_US < now
            };
            if !late {
                break;
            }
            let (idx, _) = self.video.pending.pop_front().expect("len > 1");
            let _ = codec.release_output_buffer(idx, false);
            inner.stats.dropped_frames.fetch_add(1, Ordering::Relaxed);
        }
        let due = {
            let bi = &self.video.pending[0].1;
            !inner.clock.is_valid() || bi.pts_us <= now + PRESENT_AHEAD_US
        };
        if !due {
            return;
        }
        let (idx, bi) = self.video.pending.pop_front().expect("not empty");
        let pts = bi.pts_us;
        let st = codec.release_output_buffer(idx, true);
        if st != AMEDIA_OK {
            crate::vwarn!("releaseOutputBuffer(render) failed: {st}");
        }
        inner.stats.decoded_frames.fetch_add(1, Ordering::Relaxed);
        if inner.bridge.update_tex_image() {
            let mut m = [0f32; 16];
            if inner.bridge.get_transform_matrix(&mut m) {
                self.st_matrix = m;
            }
            self.had_frame = true;
            self.frames_since_fps += 1;
            if !audio_master {
                inner.clock.set_video(pts);
            }
            if !self.first_frame_posted {
                self.first_frame_posted = true;
                inner.post(crate::api::EventType::FirstFrame, 0, 0);
            }
        }
        if self.video.output_eos && self.video.pending.is_empty() {
            inner.mark_eos(EOS_VIDEO);
        }
    }

    // ------------------------------------------------------------------ meshes

    fn ensure_mesh(&mut self, coverage: Coverage) -> bool {
        let slot = match coverage {
            Coverage::Planar => 0,
            Coverage::Full360 | Coverage::Half180 => 1,
        };
        if let (Some(_), c) = &self.meshes[slot] {
            if *c == coverage {
                return true;
            }
        }
        let mesh = mesh_for(coverage, self.mesh_step);
        match GlMesh::upload(&mesh) {
            Ok(gl) => {
                if let Some(old) = self.meshes[slot].0.take() {
                    old.delete();
                }
                crate::vlog!(
                    "uploaded {:?} mesh: {} vertices, {} indices, step {}°",
                    coverage,
                    mesh.vertex_count(),
                    mesh.index_count(),
                    self.mesh_step
                );
                self.meshes[slot] = (Some(gl), coverage);
                true
            }
            Err(e) => {
                crate::verror!("mesh upload failed: {e}");
                false
            }
        }
    }

    // -------------------------------------------------------------------- draw

    fn resolve_projection(&self, inner: &Arc<Inner>) -> ResolvedProjection {
        let mode = inner.projection_mode();
        let eye = inner.current_eye();
        let hints = inner
            .hints
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| MediaHints::default());
        let mut r = ResolvedProjection::resolve(mode, &hints, eye);
        // the user may invert the packing order advertised by the container
        let user_swap = inner.swap_eyes.load(Ordering::Relaxed);
        if user_swap != hints.swap_eyes {
            r.uv = UvRect::for_layout(r.layout, r.eye, true);
        }
        r
    }

    fn swap(&self) {
        if let Some(egl) = self.egl.as_ref() {
            let _ = egl.swap();
        }
    }

    fn draw(&mut self, inner: &Arc<Inner>) {
        let (w, h, has_window) = match self.egl.as_ref() {
            Some(e) => (e.width, e.height, e.has_window()),
            None => return,
        };
        if !has_window {
            return;
        }
        let viewport = Viewport {
            width: w.max(1) as u32,
            height: h.max(1) as u32,
        };
        inner.viewport_w.store(w, Ordering::Relaxed);
        inner.viewport_h.store(h, Ordering::Relaxed);

        let resolved = self.resolve_projection(inner);
        if self.resolution.map(|p| p != resolved).unwrap_or(true) {
            crate::vlog!("projection: {}", resolved.describe());
            inner.post(
                crate::api::EventType::ProjectionChanged,
                resolved.mode.as_i32(),
                resolved.eye.as_i32(),
            );
        }
        self.resolution = Some(resolved);

        let head = if inner.gyro.load(Ordering::Relaxed) {
            inner
                .tracker
                .lock()
                .ok()
                .and_then(|g| g.as_ref().and_then(|t| t.head_matrix()))
        } else {
            None
        };
        let view_state = inner
            .view
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| ViewState::new());
        let vu = view_state.evaluate(resolved.coverage, viewport, head);

        unsafe {
            glViewport(0, 0, w, h);
            glClear(GL_COLOR_BUFFER_BIT);
        }

        if !self.had_frame {
            if let Some(p) = &self.solid_prog {
                unsafe {
                    glUseProgram(p.id);
                    if p.u_color >= 0 {
                        glUniform4f(p.u_color, 0.0, 0.0, 0.0, 1.0);
                    }
                }
            }
            self.swap();
            return;
        }
        if !self.ensure_mesh(resolved.coverage) {
            self.swap();
            return;
        }
        let slot = match resolved.geometry() {
            Geometry::Quad => 0,
            Geometry::Sphere | Geometry::Hemisphere => 1,
        };
        let mesh = match &self.meshes[slot].0 {
            Some(m) => m,
            None => {
                self.swap();
                return;
            }
        };
        let prog = match resolved.geometry() {
            Geometry::Quad => self.rect_prog.as_ref(),
            Geometry::Sphere | Geometry::Hemisphere => self.sphere_prog.as_ref(),
        };
        let Some(prog) = prog else {
            self.swap();
            return;
        };

        let hints = inner
            .hints
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| MediaHints::default());
        let (sx, sy) = aspect_fit(resolved, viewport, self.video.output_size, hints);
        let rotation = quarter_turns(hints.rotation.max(self.video.output_rotation));
        let uv = resolved.uv;

        unsafe {
            glUseProgram(prog.id);
            glBindBuffer(GL_ARRAY_BUFFER, mesh.pos);
            if prog.a_pos >= 0 {
                glVertexAttribPointer(prog.a_pos as GLuint, 3, GL_FLOAT, 0, 0, ptr::null());
                glEnableVertexAttribArray(prog.a_pos as GLuint);
            }
            glBindBuffer(GL_ARRAY_BUFFER, mesh.uv);
            if prog.a_uv >= 0 {
                glVertexAttribPointer(prog.a_uv as GLuint, 2, GL_FLOAT, 0, 0, ptr::null());
                glEnableVertexAttribArray(prog.a_uv as GLuint);
            }
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, mesh.idx);
            if prog.u_model >= 0 {
                glUniformMatrix4fv(prog.u_model, 1, 0, vu.model.as_slice().as_ptr());
            }
            if prog.u_view >= 0 {
                glUniformMatrix4fv(prog.u_view, 1, 0, vu.view.as_slice().as_ptr());
            }
            if prog.u_proj >= 0 {
                glUniformMatrix4fv(prog.u_proj, 1, 0, vu.projection.as_slice().as_ptr());
            }
            if prog.u_uv_rect >= 0 {
                glUniform4f(prog.u_uv_rect, uv.u0, uv.v0, uv.su, uv.sv);
            }
            if prog.u_st >= 0 {
                glUniformMatrix4fv(prog.u_st, 1, 0, self.st_matrix.as_ptr());
            }
            if prog.u_scale >= 0 {
                glUniform2f(prog.u_scale, sx, sy);
            }
            if prog.u_rotation >= 0 {
                glUniform1i(prog.u_rotation, rotation);
            }
            glActiveTexture(GL_TEXTURE0);
            glBindTexture(GL_TEXTURE_EXTERNAL_OES, self.oes_tex);
            if prog.u_tex >= 0 {
                glUniform1i(prog.u_tex, 0);
            }
            glDrawElements(GL_TRIANGLES, mesh.count, GL_UNSIGNED_SHORT, ptr::null());
            let err = glGetError();
            if err != GL_NO_ERROR {
                crate::vwarn!("GL error {}", err_hex(err as i32));
            }
        }
        self.swap();
        inner.stats.rendered_frames.fetch_add(1, Ordering::Relaxed);
    }

    fn update_hud(&mut self, inner: &Arc<Inner>) {
        let viewport = Viewport {
            width: inner.viewport_w.load(Ordering::Relaxed).max(1) as u32,
            height: inner.viewport_h.load(Ordering::Relaxed).max(1) as u32,
        };
        let resolved = self
            .resolution
            .unwrap_or_else(|| self.resolve_projection(inner));
        let view_state = inner
            .view
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| ViewState::new());
        let vu = view_state.evaluate(resolved.coverage, viewport, None);
        let gyro_ready = inner
            .tracker
            .lock()
            .ok()
            .map(|g| g.as_ref().map(|t| t.is_ready()).unwrap_or(false))
            .unwrap_or(false);

        let elapsed = self.last_fps_sample.elapsed();
        if elapsed >= Duration::from_millis(500) {
            let fps = self.frames_since_fps as f32 * 1000.0 / elapsed.as_millis() as f32;
            inner
                .stats
                .render_fps_x100
                .store((fps * 100.0) as u32, Ordering::Relaxed);
            self.frames_since_fps = 0;
            self.last_fps_sample = Instant::now();
        }

        let info = inner.info.lock().map(|g| g.clone()).unwrap_or_default();
        let snap = HudSnapshot {
            yaw: vu.yaw,
            pitch: vu.pitch,
            roll: vu.roll,
            fov_y: vu.fov_y,
            fov_x: vu.fov_x,
            zoom: view_state.zoom_level(),
            mode: resolved.mode.label(),
            coverage: resolved.coverage.label(),
            layout: resolved.layout.label(),
            eye: resolved.eye.label(),
            source: resolved.source.label(),
            at_yaw_limit: vu.at_yaw_limit,
            at_pitch_limit: vu.at_pitch_limit,
            converging: vu.limits.converging,
            resistance: vu.resistance,
            gyro: inner.gyro.load(Ordering::Relaxed),
            gyro_ready,
            position_ms: (inner.clock.now_us() / 1000).max(0),
            duration_ms: inner.duration_ms(),
            video_width: info.width.max(0) as u32,
            video_height: info.height.max(0) as u32,
            rotation: info.rotation,
            fps: inner.stats.render_fps_x100.load(Ordering::Relaxed) as f32 / 100.0,
            decoded_frames: inner.stats.decoded_frames.load(Ordering::Relaxed),
            dropped_frames: inner.stats.dropped_frames.load(Ordering::Relaxed),
            buffered_ms: inner.video_q.buffered_ms().max(inner.audio_q.buffered_ms()),
            state: inner.state().label(),
            viewport_w: viewport.width,
            viewport_h: viewport.height,
            ids: [
                resolved.mode.as_i32(),
                crate::api::coverage_id(Some(resolved.coverage)),
                crate::api::layout_id(Some(resolved.layout)),
                resolved.eye.as_i32(),
                match resolved.source {
                    ProjectionSource::Forced => 0,
                    ProjectionSource::Metadata => 1,
                    ProjectionSource::Fallback => 2,
                },
            ],
        };
        // report boundary transitions once
        let flags = i32::from(vu.at_yaw_limit) | (i32::from(vu.at_pitch_limit) << 1);
        if flags != 0 && inner.boundary_posted.swap(flags, Ordering::Relaxed) != flags {
            inner.post(crate::api::EventType::BoundaryReached, flags, 0);
        } else if flags == 0 {
            inner.boundary_posted.store(0, Ordering::Relaxed);
        }

        let floats = snap.to_floats();
        if let Ok(mut g) = inner.stats.hud.lock() {
            let n = HUD_FLOAT_COUNT.min(g.len());
            g[..n].copy_from_slice(&floats[..n]);
        }
        if let Ok(mut g) = inner.stats.hud_full.lock() {
            *g = snap;
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        for slot in &self.meshes {
            if let Some(m) = &slot.0 {
                m.delete();
            }
        }
        if let Some(p) = self.sphere_prog.take() {
            p.delete();
        }
        if let Some(p) = self.rect_prog.take() {
            p.delete();
        }
        if let Some(p) = self.solid_prog.take() {
            p.delete();
        }
        if self.tex_ready {
            unsafe { glDeleteTextures(1, &self.oes_tex) };
            self.tex_ready = false;
        }
    }
}

/// Aspect fit factors for the flat quad.
fn aspect_fit(
    resolved: ResolvedProjection,
    viewport: Viewport,
    video_size: (i32, i32),
    hints: MediaHints,
) -> (f32, f32) {
    if resolved.coverage != Coverage::Planar {
        return (1.0, 1.0);
    }
    let (mut vw, mut vh) = if video_size.0 > 0 && video_size.1 > 0 {
        (video_size.0 as f32, video_size.1 as f32)
    } else {
        (hints.width as f32, hints.height as f32)
    };
    if vw <= 0.0 || vh <= 0.0 {
        return (1.0, 1.0);
    }
    let rot = quarter_turns(hints.rotation);
    if rot == 1 || rot == 3 {
        std::mem::swap(&mut vw, &mut vh);
    }
    if resolved.layout == StereoLayout::SideBySide {
        vw /= 2.0;
    } else if resolved.layout == StereoLayout::TopBottom {
        vh /= 2.0;
    }
    let screen = viewport.aspect();
    let video = vw / vh;
    if screen > video {
        (video / screen, 1.0)
    } else {
        (1.0, screen / video)
    }
}

/// Normalise a rotation in degrees to quarter turns (0…3).
pub(crate) fn quarter_turns(deg: i32) -> i32 {
    let d = deg.rem_euclid(360);
    ((d + 45) / 90).rem_euclid(4)
}

/// Software decoder component names, indexed by MIME type.
fn software_decoder(mime: &str) -> Option<String> {
    let name = match mime {
        "video/avc" => "c2.android.avc.decoder",
        "video/hevc" => "c2.android.hevc.decoder",
        "video/x-vnd.on2.vp8" => "c2.android.vp8.decoder",
        "video/x-vnd.on2.vp9" => "c2.android.vp9.decoder",
        "video/av01" => "c2.android.av1.decoder",
        "video/mp4v-es" => "c2.android.mpeg4.decoder",
        "video/3gpp" => "c2.android.h263.decoder",
        "audio/mp4a-latm" => "c2.android.aac.decoder",
        "audio/mpeg" => "c2.android.mp3.decoder",
        "audio/opus" => "c2.android.opus.decoder",
        "audio/vorbis" => "c2.android.vorbis.decoder",
        "audio/flac" => "c2.android.flac.decoder",
        "audio/g711-alaw" => "c2.android.g711.alaw.decoder",
        "audio/g711-mlaw" => "c2.android.g711.mlaw.decoder",
        _ => return None,
    };
    Some(name.to_string())
}

/// The render thread entry point.
pub(crate) fn render_thread(inner: Arc<Inner>) {
    set_thread_name("vlcrs-render");
    let _ = inner.bridge.attach_permanently();
    let mesh_step = inner
        .options
        .lock()
        .map(|o| o.mesh_step)
        .unwrap_or(vlcrs_vr::mesh::DEFAULT_STEP_DEG);
    let mut r = Renderer::new(mesh_step);

    while inner.running.load(Ordering::Acquire) {
        if inner.stop_requested.load(Ordering::Acquire) {
            r.teardown_codec();
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        // (re)attach the application surface when requested
        if inner.surface_pending.swap(false, Ordering::AcqRel) {
            if r.ensure_gl() {
                let surface = inner
                    .surface
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .and_then(|s| inner.bridge.native_window(&s));
                let egl = r.egl.as_mut().expect("ensure_gl succeeded");
                match surface {
                    Some(w) => {
                        match egl.set_window(w.as_ptr()) {
                            Ok(()) => {
                                // keep the reference alive as long as the surface
                                r.app_window = Some(w);
                            }
                            Err(e) => {
                                crate::verror!("set_window failed: {e}");
                                inner.post(
                                    crate::api::EventType::Error,
                                    crate::api::ErrorCode::RenderFailed as i32,
                                    0,
                                );
                            }
                        }
                    }
                    None => {
                        let _ = egl.clear_window();
                        r.app_window = None;
                    }
                }
            } else {
                inner.post(
                    crate::api::EventType::Error,
                    crate::api::ErrorCode::RenderFailed as i32,
                    0,
                );
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }
        }
        if !r.ensure_gl() {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }
        if inner.media_ready.load(Ordering::Acquire) && r.video.codec.is_none() {
            r.ensure_codec(&inner);
        }

        r.pump_video(&inner);
        r.present_due_frame(&inner);
        if r.egl.as_ref().map(|e| e.has_window()).unwrap_or(false) {
            r.draw(&inner);
            r.update_hud(&inner);
        } else {
            // no surface: keep the decoder draining so audio stays in sync
            std::thread::sleep(IDLE_SLEEP);
        }
        if !inner.media_ready.load(Ordering::Acquire) && r.video.codec.is_none() {
            std::thread::sleep(IDLE_SLEEP);
        }
    }

    r.teardown_codec();
    if r.video.surface.take().is_some() {
        inner.bridge.release_decoder_surface();
    }
    r.video.window = None;
    r.app_window = None;
    crate::vlog!("render thread exit");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_turns_normalises() {
        assert_eq!(quarter_turns(0), 0);
        assert_eq!(quarter_turns(90), 1);
        assert_eq!(quarter_turns(180), 2);
        assert_eq!(quarter_turns(270), 3);
        assert_eq!(quarter_turns(360), 0);
        assert_eq!(quarter_turns(-90), 3);
        assert_eq!(quarter_turns(44), 0);
        assert_eq!(quarter_turns(46), 1);
    }

    #[test]
    fn aspect_fit_letterboxes_and_pillarboxes() {
        let viewport = Viewport {
            width: 2400,
            height: 1080,
        };
        let hints = MediaHints {
            width: 1920,
            height: 1080,
            ..Default::default()
        };
        let planar = ResolvedProjection::resolve(
            vlcrs_vr::ProjectionMode::Planar,
            &hints,
            vlcrs_vr::Eye::Left,
        );
        // a 16:9 picture on a 20:9 surface is pillarboxed
        let (sx, sy) = aspect_fit(planar, viewport, (0, 0), hints);
        assert!(sy == 1.0 && sx < 1.0, "({sx}, {sy})");
        assert!((sx - (16.0 / 9.0) / viewport.aspect()).abs() < 1e-4);

        // a 4:3 picture is letterboxed
        let tall = MediaHints {
            width: 1440,
            height: 1080,
            ..Default::default()
        };
        let (sx, sy) = aspect_fit(planar, viewport, (0, 0), tall);
        assert!(sx == 1.0 && sy < 1.0, "({sx}, {sy})");

        // spherical modes are never scaled
        let sphere = ResolvedProjection::resolve(
            vlcrs_vr::ProjectionMode::E360Mono,
            &hints,
            vlcrs_vr::Eye::Left,
        );
        assert_eq!(aspect_fit(sphere, viewport, (0, 0), hints), (1.0, 1.0));
    }

    #[test]
    fn aspect_fit_accounts_for_rotation_and_stereo_halves() {
        let viewport = Viewport {
            width: 1080,
            height: 2400,
        };
        // portrait screen, landscape picture rotated 90° → fills the height
        let hints = MediaHints {
            width: 1920,
            height: 1080,
            rotation: 90,
            ..Default::default()
        };
        let planar = ResolvedProjection::resolve(
            vlcrs_vr::ProjectionMode::Planar,
            &hints,
            vlcrs_vr::Eye::Left,
        );
        let (sx, sy) = aspect_fit(planar, viewport, (0, 0), hints);
        // after rotation the picture is 1080x1920 on a 1080x2400 screen
        assert!(sx == 1.0, "sx = {sx}");
        assert!(sy < 1.0, "sy = {sy}");

        // planar rendering always uses the whole frame (mono)
        let sbs = ResolvedProjection::resolve(
            vlcrs_vr::ProjectionMode::Planar,
            &MediaHints {
                width: 3840,
                height: 1080,
                ..Default::default()
            },
            vlcrs_vr::Eye::Left,
        );
        assert_eq!(sbs.layout, StereoLayout::Mono, "planar forces mono");
        assert_eq!(sbs.uv, UvRect::FULL);
    }

    #[test]
    fn spherical_uv_rects_select_one_eye() {
        let hints = MediaHints {
            width: 3840,
            height: 1920,
            ..Default::default()
        };
        for (mode, expect_su, expect_sv) in [
            (vlcrs_vr::ProjectionMode::E360Mono, 1.0f32, 1.0f32),
            (vlcrs_vr::ProjectionMode::E360Sbs, 0.5, 1.0),
            (vlcrs_vr::ProjectionMode::E180Tb, 1.0, 0.5),
        ] {
            let r = ResolvedProjection::resolve(mode, &hints, vlcrs_vr::Eye::Left);
            assert_eq!((r.uv.su, r.uv.sv), (expect_su, expect_sv), "{mode}");
        }
        let left = ResolvedProjection::resolve(
            vlcrs_vr::ProjectionMode::E180Sbs,
            &hints,
            vlcrs_vr::Eye::Left,
        );
        let right = ResolvedProjection::resolve(
            vlcrs_vr::ProjectionMode::E180Sbs,
            &hints,
            vlcrs_vr::Eye::Right,
        );
        assert_eq!(left.uv.u0, 0.0);
        assert_eq!(right.uv.u0, 0.5);
        assert_eq!(
            left.coverage, right.coverage,
            "eye switch keeps the geometry"
        );
    }
}
