//! GLSL ES 1.00 sources for the libvlcrs renderer.
//!
//! Only the paths that the engine actually uses are provided:
//!
//! * [`VS_SPHERE`] + [`FS_OES`] — spherical (360°/180°) projection of an
//!   external OES texture (the `MediaCodec` output surface);
//! * [`VS_RECT`] + [`FS_OES`] — forced planar 2D rendering.
//!
//! The eye/layout sub rectangle is applied in the vertex shader through
//! `uUvRect = (u0, v0, su, sv)` so that switching the format or the eye is a
//! single uniform update: no re-upload, no pipeline restart, the playback
//! position is untouched.  The `SurfaceTexture` transform matrix (`uStMatrix`)
//! is applied in the fragment shader, exactly like the reference player.

/// Vertex shader for the spherical geometries (full sphere / hemisphere).
pub const VS_SPHERE: &str = r#"
attribute vec3 aPosition;
attribute vec2 aTexCoord;
uniform mat4 uModelMatrix;
uniform mat4 uViewMatrix;
uniform mat4 uProjectionMatrix;
uniform vec4 uUvRect;
varying vec2 vUv;
void main() {
    vUv = uUvRect.xy + aTexCoord * uUvRect.zw;
    gl_Position = uProjectionMatrix * uViewMatrix * uModelMatrix * vec4(aPosition, 1.0);
}
"#;

/// Vertex shader for the flat quad (forced planar mode).
///
/// `uScale.xy` implements the aspect-ratio fit (letterbox) and `uRotation` is
/// the display rotation of the picture in quarter turns (0…3), matching the
/// reference `vs_rect`.
pub const VS_RECT: &str = r#"
attribute vec3 aPosition;
attribute vec2 aTexCoord;
uniform vec2 uScale;
uniform int uRotation;
uniform vec4 uUvRect;
varying vec2 vUv;

const mat2 ROT90  = mat2(0.0, -1.0, 1.0, 0.0);
const mat2 ROT180 = mat2(-1.0, 0.0, 0.0, -1.0);
const mat2 ROT270 = mat2(0.0, 1.0, -1.0, 0.0);

void main() {
    vUv = uUvRect.xy + aTexCoord * uUvRect.zw;
    vec2 xy = aPosition.xy * uScale;
    if (uRotation == 1) {
        xy = ROT90 * xy;
    } else if (uRotation == 2) {
        xy = ROT180 * xy;
    } else if (uRotation == 3) {
        xy = ROT270 * xy;
    }
    gl_Position = vec4(xy, aPosition.z, 1.0);
}
"#;

/// Fragment shader sampling an external OES texture (hardware decoder output).
pub const FS_OES: &str = r#"
#extension GL_OES_EGL_image_external : require
precision mediump float;
uniform samplerExternalOES uTexture;
uniform mat4 uStMatrix;
varying vec2 vUv;
void main() {
    vec2 tc = (uStMatrix * vec4(vUv, 0.0, 1.0)).xy;
    gl_FragColor = texture2D(uTexture, tc);
}
"#;

/// Fragment shader used before the first frame is available: paints the
/// background so that the surface is not showing stale content.
pub const VS_FLAT: &str = r#"
attribute vec3 aPosition;
void main() {
    gl_Position = vec4(aPosition, 1.0);
}
"#;

/// Solid colour fragment shader (background / "no video yet").
pub const FS_SOLID: &str = r#"
precision mediump float;
uniform vec4 uColor;
void main() {
    gl_FragColor = uColor;
}
"#;

/// Attribute names, kept in one place so the Rust side cannot drift.
pub mod attrib {
    /// Vertex position attribute.
    pub const POSITION: &str = "aPosition";
    /// Texture coordinate attribute.
    pub const TEXCOORD: &str = "aTexCoord";
}

/// Uniform names.
pub mod uniform {
    /// Model matrix (sphere orientation).
    pub const MODEL: &str = "uModelMatrix";
    /// View matrix.
    pub const VIEW: &str = "uViewMatrix";
    /// Projection matrix.
    pub const PROJECTION: &str = "uProjectionMatrix";
    /// Eye/layout sub rectangle `(u0, v0, su, sv)`.
    pub const UV_RECT: &str = "uUvRect";
    /// `SurfaceTexture` transform matrix.
    pub const ST_MATRIX: &str = "uStMatrix";
    /// Video texture (external OES).
    pub const TEXTURE: &str = "uTexture";
    /// Aspect fit scale for the flat quad.
    pub const SCALE: &str = "uScale";
    /// Quarter turn rotation for the flat quad.
    pub const ROTATION: &str = "uRotation";
    /// Solid colour.
    pub const COLOR: &str = "uColor";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaders_declare_every_uniform_the_renderer_sets() {
        for (src, names) in [
            (
                VS_SPHERE,
                [
                    uniform::MODEL,
                    uniform::VIEW,
                    uniform::PROJECTION,
                    uniform::UV_RECT,
                    attrib::POSITION,
                    attrib::TEXCOORD,
                ]
                .as_slice(),
            ),
            (
                VS_RECT,
                [uniform::SCALE, uniform::ROTATION, uniform::UV_RECT].as_slice(),
            ),
            (FS_OES, [uniform::TEXTURE, uniform::ST_MATRIX].as_slice()),
        ] {
            for n in names {
                assert!(src.contains(n), "shader is missing `{n}`");
            }
        }
    }

    #[test]
    fn oes_shader_requires_the_extension() {
        assert!(FS_OES.contains("GL_OES_EGL_image_external"));
        assert!(FS_OES.contains("samplerExternalOES"));
    }

    #[test]
    fn shaders_are_gles2_compatible() {
        // No GLES3-only keywords: the renderer targets an ES2 context so that
        // it works on every arm64 device we care about.
        for src in [VS_SPHERE, VS_RECT, FS_OES, VS_FLAT, FS_SOLID] {
            assert!(!src.contains("#version 3"), "must stay GLSL ES 1.00");
            assert!(!src.contains(" in vec"), "no GLES3 `in` qualifiers");
            assert!(!src.contains("texture("), "use texture2D on ES2");
        }
    }

    #[test]
    fn uv_rect_semantics_are_documented_in_the_shader() {
        // vUv = origin + coord * scale : cropping happens *before* the
        // SurfaceTexture matrix so the eye halves are defined in display space.
        assert!(VS_SPHERE.contains("uUvRect.xy + aTexCoord * uUvRect.zw"));
        assert!(FS_OES.contains("uStMatrix * vec4(vUv"));
    }
}
