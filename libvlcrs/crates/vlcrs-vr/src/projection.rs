//! The **format matrix**: coverage × stereo layout, plus `auto` and `planar`.
//!
//! | mode | coverage | layout |
//! |------|----------|--------|
//! | `Auto` | from metadata | from metadata |
//! | `Planar` | flat 2D quad | mono (whole frame) |
//! | `E360Mono` | 360° | mono |
//! | `E360Sbs` | 360° | side by side |
//! | `E360Tb` | 360° | top bottom |
//! | `E180Mono` | 180° | mono |
//! | `E180Sbs` | 180° | side by side |
//! | `E180Tb` | 180° | top bottom |
//!
//! Every explicit mode can be forced onto *any* source (requirement: the user
//! may render an ordinary 2D file as 360°/180° SBS/TB), and resolving a mode is
//! a pure function — switching it mid-playback therefore never touches the
//! decode pipeline and the playback position is preserved.

use std::fmt;

/// Horizontal coverage of the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Coverage {
    /// Flat 2D picture (forced planar mode).
    Planar,
    /// Full 360° equirectangular sphere.
    Full360,
    /// 180° equirectangular hemisphere (front half only).
    Half180,
}

impl Coverage {
    /// Half of the horizontal coverage in degrees (`180`, `90` or `0`).
    pub fn yaw_extent_deg(self) -> f32 {
        match self {
            Coverage::Planar => 0.0,
            Coverage::Full360 => 180.0,
            Coverage::Half180 => 90.0,
        }
    }

    /// Half of the vertical coverage in degrees.
    pub fn pitch_extent_deg(self) -> f32 {
        match self {
            Coverage::Planar => 0.0,
            Coverage::Full360 | Coverage::Half180 => 90.0,
        }
    }

    /// `true` for the spherical coverages.
    pub fn is_spherical(self) -> bool {
        !matches!(self, Coverage::Planar)
    }

    /// `true` when yaw wraps around instead of hitting a boundary.
    pub fn wraps_horizontally(self) -> bool {
        matches!(self, Coverage::Full360)
    }

    /// Short human readable name for the HUD.
    pub fn label(self) -> &'static str {
        match self {
            Coverage::Planar => "2D",
            Coverage::Full360 => "360°",
            Coverage::Half180 => "180°",
        }
    }
}

/// How the two eyes are packed inside the decoded picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StereoLayout {
    /// Single eye picture, the whole frame is used.
    Mono,
    /// Left/right halves.
    SideBySide,
    /// Top/bottom halves.
    TopBottom,
}

impl StereoLayout {
    /// Short human readable name for the HUD.
    pub fn label(self) -> &'static str {
        match self {
            StereoLayout::Mono => "mono",
            StereoLayout::SideBySide => "SBS",
            StereoLayout::TopBottom => "TB",
        }
    }

    /// `true` when only half of the frame is used.
    pub fn is_stereo(self) -> bool {
        !matches!(self, StereoLayout::Mono)
    }
}

/// Which eye of a stereo layout gets rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Eye {
    /// Left eye (also the only meaningful value for mono sources).
    Left,
    /// Right eye.
    Right,
}

impl Eye {
    /// The other eye.
    pub fn other(self) -> Eye {
        match self {
            Eye::Left => Eye::Right,
            Eye::Right => Eye::Left,
        }
    }

    /// Short human readable name for the HUD.
    pub fn label(self) -> &'static str {
        match self {
            Eye::Left => "L",
            Eye::Right => "R",
        }
    }

    /// Decode from the JNI integer representation.
    pub fn from_i32(v: i32) -> Eye {
        if v == 1 {
            Eye::Right
        } else {
            Eye::Left
        }
    }

    /// Integer representation used across the JNI boundary.
    pub fn as_i32(self) -> i32 {
        match self {
            Eye::Left => 0,
            Eye::Right => 1,
        }
    }
}

/// User selectable rendering mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProjectionMode {
    /// Derive coverage/layout from the container metadata.
    Auto,
    /// Force flat 2D rendering (any source).
    Planar,
    /// 360° equirectangular, single eye.
    E360Mono,
    /// 360° equirectangular, side by side stereo.
    E360Sbs,
    /// 360° equirectangular, top bottom stereo.
    E360Tb,
    /// 180° equirectangular hemisphere, single eye.
    E180Mono,
    /// 180° equirectangular hemisphere, side by side stereo.
    E180Sbs,
    /// 180° equirectangular hemisphere, top bottom stereo.
    E180Tb,
}

/// Every mode, in the order the UI should present them.
pub const ALL_MODES: [ProjectionMode; 8] = [
    ProjectionMode::Auto,
    ProjectionMode::Planar,
    ProjectionMode::E360Mono,
    ProjectionMode::E360Sbs,
    ProjectionMode::E360Tb,
    ProjectionMode::E180Mono,
    ProjectionMode::E180Sbs,
    ProjectionMode::E180Tb,
];

impl ProjectionMode {
    /// Integer representation used across the JNI boundary. Stable ABI.
    pub fn as_i32(self) -> i32 {
        match self {
            ProjectionMode::Auto => 0,
            ProjectionMode::Planar => 1,
            ProjectionMode::E360Mono => 2,
            ProjectionMode::E360Sbs => 3,
            ProjectionMode::E360Tb => 4,
            ProjectionMode::E180Mono => 5,
            ProjectionMode::E180Sbs => 6,
            ProjectionMode::E180Tb => 7,
        }
    }

    /// Decode the JNI integer representation; unknown values fall back to `Auto`.
    pub fn from_i32(v: i32) -> ProjectionMode {
        ALL_MODES
            .into_iter()
            .find(|m| m.as_i32() == v)
            .unwrap_or(ProjectionMode::Auto)
    }

    /// Short name for menus/HUD.
    pub fn label(self) -> &'static str {
        match self {
            ProjectionMode::Auto => "Auto",
            ProjectionMode::Planar => "Planar",
            ProjectionMode::E360Mono => "360 Mono",
            ProjectionMode::E360Sbs => "360 SBS",
            ProjectionMode::E360Tb => "360 TB",
            ProjectionMode::E180Mono => "180 Mono",
            ProjectionMode::E180Sbs => "180 SBS",
            ProjectionMode::E180Tb => "180 TB",
        }
    }

    /// Explicit coverage, if the mode is not `Auto`.
    pub fn coverage(self) -> Option<Coverage> {
        match self {
            ProjectionMode::Auto => None,
            ProjectionMode::Planar => Some(Coverage::Planar),
            ProjectionMode::E360Mono | ProjectionMode::E360Sbs | ProjectionMode::E360Tb => {
                Some(Coverage::Full360)
            }
            ProjectionMode::E180Mono | ProjectionMode::E180Sbs | ProjectionMode::E180Tb => {
                Some(Coverage::Half180)
            }
        }
    }

    /// Explicit stereo layout, if the mode is not `Auto`.
    pub fn layout(self) -> Option<StereoLayout> {
        match self {
            ProjectionMode::Auto => None,
            ProjectionMode::Planar | ProjectionMode::E360Mono | ProjectionMode::E180Mono => {
                Some(StereoLayout::Mono)
            }
            ProjectionMode::E360Sbs | ProjectionMode::E180Sbs => Some(StereoLayout::SideBySide),
            ProjectionMode::E360Tb | ProjectionMode::E180Tb => Some(StereoLayout::TopBottom),
        }
    }

    /// `true` when the mode needs no metadata.
    pub fn is_explicit(self) -> bool {
        !matches!(self, ProjectionMode::Auto)
    }
}

impl fmt::Display for ProjectionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A sub rectangle of the picture, in display space (`v = 0` at the bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvRect {
    /// Horizontal origin.
    pub u0: f32,
    /// Vertical origin.
    pub v0: f32,
    /// Horizontal scale.
    pub su: f32,
    /// Vertical scale.
    pub sv: f32,
}

impl UvRect {
    /// The whole picture.
    pub const FULL: UvRect = UvRect {
        u0: 0.0,
        v0: 0.0,
        su: 1.0,
        sv: 1.0,
    };

    /// Sub rectangle for one eye of a stereo layout.
    ///
    /// `swap_eyes` inverts the packing order: by default the left eye is the
    /// left half (side-by-side) or the top half (top-bottom), matching
    /// `st3d = 2` / Matroska `StereoMode = 1|3`.  `st3d = 4` (right-left) and
    /// Matroska `StereoMode = 2|11` set `swap_eyes`, i.e. the first half holds
    /// the *right* eye.
    pub fn for_layout(layout: StereoLayout, eye: Eye, swap_eyes: bool) -> UvRect {
        // `first` = the eye stored in the first (left / top) half
        let first_is_left = !swap_eyes;
        let in_first_half = (eye == Eye::Left) == first_is_left;
        match layout {
            StereoLayout::Mono => UvRect::FULL,
            StereoLayout::SideBySide => UvRect {
                u0: if in_first_half { 0.0 } else { 0.5 },
                v0: 0.0,
                su: 0.5,
                sv: 1.0,
            },
            StereoLayout::TopBottom => UvRect {
                u0: 0.0,
                // Display space: v = 0 is the *bottom* of the picture, so the
                // first (top) half is v ∈ [0.5, 1].
                v0: if in_first_half { 0.5 } else { 0.0 },
                su: 1.0,
                sv: 0.5,
            },
        }
    }

    /// `[u0, v0, su, sv]` for the shader uniform.
    pub fn as_array(self) -> [f32; 4] {
        [self.u0, self.v0, self.su, self.sv]
    }
}

/// Initial viewing direction stored in the container (`prhd` / Matroska
/// `ProjectionPose*`), in degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
    /// Roll in degrees.
    pub roll: f32,
}

/// What the demuxer/prober knows about the source.
#[derive(Clone, Copy, Debug, Default)]
pub struct MediaHints {
    /// Coded width in pixels.
    pub width: u32,
    /// Coded height in pixels.
    pub height: u32,
    /// Display rotation in degrees (0/90/180/270).
    pub rotation: i32,
    /// Stereo layout advertised by the container, if any.
    pub stereo: Option<StereoLayout>,
    /// `true` when the container says the *first* half (left for SBS, top for
    /// TB) holds the right eye (`st3d = 4`, Matroska `StereoMode = 2|11`).
    pub swap_eyes: bool,
    /// Coverage advertised by the container, if any.
    pub coverage: Option<Coverage>,
    /// Initial pose advertised by the container, if any.
    pub pose: Option<Pose>,
}

impl MediaHints {
    /// Hints that carry no metadata at all.
    pub fn unknown() -> Self {
        Self::default()
    }

    /// `true` when the container advertised spherical metadata.
    pub fn has_metadata(&self) -> bool {
        self.stereo.is_some() || self.coverage.is_some()
    }

    /// Aspect ratio (`width / height`), or `0` when unknown.
    pub fn aspect(&self) -> f32 {
        if self.height == 0 {
            0.0
        } else {
            self.width as f32 / self.height as f32
        }
    }

    /// Pure heuristic (never used by `Auto`, only surfaced to the UI): a 2:1
    /// picture usually is a full equirect sphere, a 1:1 picture a 180° one.
    pub fn aspect_suggestion(&self) -> Option<Coverage> {
        let a = self.aspect();
        if a <= 0.0 {
            return None;
        }
        if (a - 2.0).abs() < 0.06 {
            Some(Coverage::Full360)
        } else if (a - 1.0).abs() < 0.06 {
            Some(Coverage::Half180)
        } else {
            None
        }
    }
}

/// Where the resolved projection came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionSource {
    /// Explicitly selected by the user.
    Forced,
    /// Derived from container metadata (`Auto`).
    Metadata,
    /// No metadata: `Auto` fell back to flat 2D.
    Fallback,
}

impl ProjectionSource {
    /// Short human readable name for the HUD.
    pub fn label(self) -> &'static str {
        match self {
            ProjectionSource::Forced => "forced",
            ProjectionSource::Metadata => "metadata",
            ProjectionSource::Fallback => "fallback",
        }
    }
}

/// Fully resolved rendering parameters for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedProjection {
    /// Geometry to draw.
    pub coverage: Coverage,
    /// Stereo layout of the source.
    pub layout: StereoLayout,
    /// Eye being rendered.
    pub eye: Eye,
    /// Sub rectangle of the picture used for that eye.
    pub uv: UvRect,
    /// Where the decision came from.
    pub source: ProjectionSource,
    /// Mode that produced this resolution (useful for the HUD).
    pub mode: ProjectionMode,
}

impl ResolvedProjection {
    /// Resolve a mode against the metadata of the current item.
    pub fn resolve(mode: ProjectionMode, hints: &MediaHints, eye: Eye) -> ResolvedProjection {
        let (coverage, layout, source) = match mode {
            ProjectionMode::Auto => {
                let cov = match hints.coverage {
                    Some(c) => c,
                    // Stereo packing without an explicit projection is spherical
                    // by definition; 360° is the common case.
                    None if hints.stereo.is_some_and(StereoLayout::is_stereo) => Coverage::Full360,
                    None => Coverage::Planar,
                };
                let layout = match cov {
                    Coverage::Planar => StereoLayout::Mono,
                    _ => hints.stereo.unwrap_or(StereoLayout::Mono),
                };
                let source = if hints.has_metadata() {
                    ProjectionSource::Metadata
                } else {
                    ProjectionSource::Fallback
                };
                (cov, layout, source)
            }
            m => (
                m.coverage().unwrap_or(Coverage::Planar),
                m.layout().unwrap_or(StereoLayout::Mono),
                ProjectionSource::Forced,
            ),
        };
        // A flat picture has no eye to choose from.
        let layout = if coverage == Coverage::Planar {
            StereoLayout::Mono
        } else {
            layout
        };
        ResolvedProjection {
            coverage,
            layout,
            eye,
            uv: UvRect::for_layout(layout, eye, hints.swap_eyes),
            source,
            mode,
        }
    }

    /// Geometry selection for the renderer.
    pub fn geometry(&self) -> Geometry {
        match self.coverage {
            Coverage::Planar => Geometry::Quad,
            Coverage::Full360 => Geometry::Sphere,
            Coverage::Half180 => Geometry::Hemisphere,
        }
    }

    /// One line description for the HUD / logs.
    pub fn describe(&self) -> String {
        let eye = if self.layout.is_stereo() {
            format!(" eye={}", self.eye.label())
        } else {
            String::new()
        };
        format!(
            "{} {} {}{} ({})",
            self.mode.label(),
            self.coverage.label(),
            self.layout.label(),
            eye,
            self.source.label()
        )
    }
}

/// Which geometry the renderer must upload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Geometry {
    /// Flat quad (2D).
    Quad,
    /// Full sphere.
    Sphere,
    /// Front hemisphere.
    Hemisphere,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(stereo: Option<StereoLayout>, coverage: Option<Coverage>) -> MediaHints {
        MediaHints {
            width: 3840,
            height: 1920,
            rotation: 0,
            stereo,
            swap_eyes: false,
            coverage,
            pose: None,
        }
    }

    #[test]
    fn swapped_eye_order_flips_the_halves() {
        // st3d = 4 (right-left) / Matroska StereoMode = 11
        let l = UvRect::for_layout(StereoLayout::SideBySide, Eye::Left, true);
        let r = UvRect::for_layout(StereoLayout::SideBySide, Eye::Right, true);
        assert_eq!((l.u0, l.su), (0.5, 0.5));
        assert_eq!((r.u0, r.su), (0.0, 0.5));
        // Matroska StereoMode = 2: top-bottom, right eye first (on top)
        let l = UvRect::for_layout(StereoLayout::TopBottom, Eye::Left, true);
        let r = UvRect::for_layout(StereoLayout::TopBottom, Eye::Right, true);
        assert_eq!((l.v0, l.sv), (0.0, 0.5));
        assert_eq!((r.v0, r.sv), (0.5, 0.5));
    }

    #[test]
    fn resolve_honours_the_metadata_eye_order() {
        let mut h = hints(Some(StereoLayout::TopBottom), Some(Coverage::Full360));
        h.swap_eyes = true;
        let r = ResolvedProjection::resolve(ProjectionMode::Auto, &h, Eye::Left);
        assert_eq!(r.uv.v0, 0.0, "left eye is at the bottom when swapped");
    }

    #[test]
    fn mode_ids_are_stable() {
        // The JNI ABI depends on these numbers.
        assert_eq!(ProjectionMode::Auto.as_i32(), 0);
        assert_eq!(ProjectionMode::Planar.as_i32(), 1);
        assert_eq!(ProjectionMode::E360Mono.as_i32(), 2);
        assert_eq!(ProjectionMode::E360Sbs.as_i32(), 3);
        assert_eq!(ProjectionMode::E360Tb.as_i32(), 4);
        assert_eq!(ProjectionMode::E180Mono.as_i32(), 5);
        assert_eq!(ProjectionMode::E180Sbs.as_i32(), 6);
        assert_eq!(ProjectionMode::E180Tb.as_i32(), 7);
        for m in ALL_MODES {
            assert_eq!(ProjectionMode::from_i32(m.as_i32()), m);
        }
        assert_eq!(
            ProjectionMode::from_i32(4242),
            ProjectionMode::Auto,
            "unknown ids must be safe"
        );
    }

    #[test]
    fn full_matrix_is_addressable() {
        // 2 coverages × 3 layouts + planar + auto == 8 modes
        let mut seen = std::collections::HashSet::new();
        for m in ALL_MODES {
            seen.insert((m.coverage(), m.layout()));
        }
        assert_eq!(seen.len(), 8);
    }

    #[test]
    fn explicit_modes_ignore_metadata() {
        let h = hints(Some(StereoLayout::TopBottom), Some(Coverage::Half180));
        let r = ResolvedProjection::resolve(ProjectionMode::E360Sbs, &h, Eye::Right);
        assert_eq!(r.coverage, Coverage::Full360);
        assert_eq!(r.layout, StereoLayout::SideBySide);
        assert_eq!(r.source, ProjectionSource::Forced);
        assert_eq!(
            r.uv,
            UvRect {
                u0: 0.5,
                v0: 0.0,
                su: 0.5,
                sv: 1.0
            }
        );
    }

    #[test]
    fn planar_forces_mono_and_full_frame() {
        let h = hints(Some(StereoLayout::SideBySide), Some(Coverage::Full360));
        let r = ResolvedProjection::resolve(ProjectionMode::Planar, &h, Eye::Right);
        assert_eq!(r.coverage, Coverage::Planar);
        assert_eq!(r.layout, StereoLayout::Mono);
        assert_eq!(r.uv, UvRect::FULL);
        assert_eq!(r.geometry(), Geometry::Quad);
    }

    #[test]
    fn auto_uses_metadata() {
        let h = hints(Some(StereoLayout::TopBottom), Some(Coverage::Half180));
        let r = ResolvedProjection::resolve(ProjectionMode::Auto, &h, Eye::Left);
        assert_eq!(r.coverage, Coverage::Half180);
        assert_eq!(r.layout, StereoLayout::TopBottom);
        assert_eq!(r.source, ProjectionSource::Metadata);
        assert_eq!(r.geometry(), Geometry::Hemisphere);
    }

    #[test]
    fn auto_without_metadata_falls_back_to_planar() {
        let r =
            ResolvedProjection::resolve(ProjectionMode::Auto, &MediaHints::unknown(), Eye::Left);
        assert_eq!(r.coverage, Coverage::Planar);
        assert_eq!(r.source, ProjectionSource::Fallback);
        assert_eq!(r.layout, StereoLayout::Mono);
    }

    #[test]
    fn auto_with_only_stereo_metadata_assumes_360() {
        let h = hints(Some(StereoLayout::SideBySide), None);
        let r = ResolvedProjection::resolve(ProjectionMode::Auto, &h, Eye::Left);
        assert_eq!(r.coverage, Coverage::Full360);
        assert_eq!(r.source, ProjectionSource::Metadata);
    }

    #[test]
    fn auto_with_only_projection_metadata_assumes_mono() {
        let h = hints(None, Some(Coverage::Full360));
        let r = ResolvedProjection::resolve(ProjectionMode::Auto, &h, Eye::Right);
        assert_eq!(r.layout, StereoLayout::Mono);
        assert_eq!(r.uv, UvRect::FULL);
    }

    #[test]
    fn sbs_eye_selection() {
        let l = UvRect::for_layout(StereoLayout::SideBySide, Eye::Left, false);
        let r = UvRect::for_layout(StereoLayout::SideBySide, Eye::Right, false);
        assert_eq!((l.u0, l.su), (0.0, 0.5));
        assert_eq!((r.u0, r.su), (0.5, 0.5));
        assert_eq!(l.sv, 1.0);
    }

    #[test]
    fn tb_eye_selection_left_is_top() {
        let l = UvRect::for_layout(StereoLayout::TopBottom, Eye::Left, false);
        let r = UvRect::for_layout(StereoLayout::TopBottom, Eye::Right, false);
        // display space: v0 = 0.5 is the top half
        assert_eq!((l.v0, l.sv), (0.5, 0.5));
        assert_eq!((r.v0, r.sv), (0.0, 0.5));
        assert_eq!(l.su, 1.0);
    }

    #[test]
    fn tb_eye_order_can_be_inverted() {
        let l = UvRect::for_layout(StereoLayout::TopBottom, Eye::Left, true);
        assert_eq!((l.v0, l.sv), (0.0, 0.5));
    }

    #[test]
    fn eye_switch_changes_only_the_rect() {
        let h = hints(Some(StereoLayout::SideBySide), Some(Coverage::Full360));
        let a = ResolvedProjection::resolve(ProjectionMode::Auto, &h, Eye::Left);
        let b = ResolvedProjection::resolve(ProjectionMode::Auto, &h, Eye::Right);
        assert_eq!(a.coverage, b.coverage);
        assert_eq!(a.layout, b.layout);
        assert_ne!(a.uv, b.uv);
        assert_eq!(b.eye, Eye::Right);
    }

    #[test]
    fn coverage_extents() {
        assert_eq!(Coverage::Full360.yaw_extent_deg(), 180.0);
        assert_eq!(Coverage::Half180.yaw_extent_deg(), 90.0);
        assert!(Coverage::Full360.wraps_horizontally());
        assert!(!Coverage::Half180.wraps_horizontally());
        assert_eq!(Coverage::Planar.pitch_extent_deg(), 0.0);
    }

    #[test]
    fn aspect_suggestion_is_only_a_hint() {
        let mut h = MediaHints::unknown();
        h.width = 3840;
        h.height = 1920;
        assert_eq!(h.aspect_suggestion(), Some(Coverage::Full360));
        h.width = 1920;
        h.height = 1920;
        assert_eq!(h.aspect_suggestion(), Some(Coverage::Half180));
        h.width = 1920;
        h.height = 1080;
        assert_eq!(h.aspect_suggestion(), None);
        // Auto still falls back to planar without metadata
        let r = ResolvedProjection::resolve(ProjectionMode::Auto, &h, Eye::Left);
        assert_eq!(r.coverage, Coverage::Planar);
    }

    #[test]
    fn describe_is_human_readable() {
        let h = hints(Some(StereoLayout::SideBySide), Some(Coverage::Full360));
        let r = ResolvedProjection::resolve(ProjectionMode::Auto, &h, Eye::Right);
        let d = r.describe();
        assert!(d.contains("360"), "{d}");
        assert!(d.contains("SBS"), "{d}");
        assert!(d.contains("R"), "{d}");
        assert!(d.contains("metadata"), "{d}");
    }
}
