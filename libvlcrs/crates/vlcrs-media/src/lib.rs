//! `vlcrs-media` — lightweight container probing for libvlcrs.
//!
//! The engine deliberately does **not** link against a demuxing library: on
//! Android the platform `AMediaExtractor`/`AMediaCodec` pair already handles
//! containers and codecs.  What the platform does *not* give us is the
//! spherical metadata needed by the `Auto` projection mode, so this crate
//! implements just enough of ISO-BMFF (MP4/MOV) and Matroska/WebM to read:
//!
//! * track list, codec ids, picture size, rotation, duration;
//! * Spherical Video **V2** boxes: `st3d` (stereo layout), `sv3d`/`proj`,
//!   `prhd` (initial pose), `equi` (equirectangular + crop bounds), `cbmp`,
//!   `mshp`;
//! * Spherical Video **V1** metadata: the `uuid` box with UUID
//!   `ffcc8263-f855-4a93-8814-587a02521fdd` and its XML payload;
//! * Matroska `StereoMode` and the `Projection` master (`ProjectionType`,
//!   `ProjectionPrivate`, `ProjectionPose*`).
//!
//! Everything is streaming (a `Read + Seek`), bounded (byte budget, depth and
//! element count limits) and panic free: a truncated or hostile file yields
//! [`ProbeError`] or simply fewer hints.

#![deny(missing_docs)]

pub mod bmff;
pub mod ebml;

use vlcrs_vr::{Coverage, MediaHints, Pose, StereoLayout};

/// Container family detected from the file header.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Container {
    /// ISO base media file format (`.mp4`, `.mov`, `.m4a`, …).
    Mp4,
    /// Matroska / WebM.
    Matroska,
    /// Not recognised; the platform demuxer may still play it.
    #[default]
    Unknown,
}

impl Container {
    /// Short label for logs/HUD.
    pub fn label(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Matroska => "mkv",
            Container::Unknown => "unknown",
        }
    }
}

/// Kind of a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TrackKind {
    /// Video track.
    Video,
    /// Audio track.
    Audio,
    /// Subtitle / text track.
    Subtitle,
    /// Anything else.
    #[default]
    Other,
}

/// Which spherical metadata scheme was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SphericalVersion {
    /// Spherical Video V1 (`uuid` box + XML).
    V1Xml,
    /// Spherical Video V2 (`st3d`/`sv3d`).
    V2Boxes,
    /// Matroska `StereoMode` / `Projection`.
    Matroska,
}

impl SphericalVersion {
    /// Short label for logs.
    pub fn label(self) -> &'static str {
        match self {
            SphericalVersion::V1Xml => "v1-xml",
            SphericalVersion::V2Boxes => "v2",
            SphericalVersion::Matroska => "mkv",
        }
    }
}

/// Projection kind advertised by the container.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ProjectionKind {
    /// Ordinary flat picture.
    Rectangular,
    /// Equirectangular (with optional crop bounds).
    Equirectangular,
    /// Cubemap.
    Cubemap,
    /// Arbitrary mesh — not supported by libvlcrs.
    Mesh,
    /// Unknown / unparsed.
    #[default]
    Unknown,
}

impl ProjectionKind {
    /// `true` when the projection can be rendered by libvlcrs.
    pub fn supported(self) -> bool {
        matches!(
            self,
            ProjectionKind::Rectangular | ProjectionKind::Equirectangular
        )
    }
}

/// Fraction of the projection cropped away on each edge (`equi` bounds /
/// Matroska `ProjectionPrivate`), each in `0.0 … 1.0`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Crop {
    /// Cropped from the top.
    pub top: f32,
    /// Cropped from the bottom.
    pub bottom: f32,
    /// Cropped from the left.
    pub cropped_left: f32,
    /// Cropped from the right.
    pub right: f32,
}

impl Crop {
    /// Horizontal fraction of the sphere that is actually covered.
    pub fn horizontal_fraction(&self) -> f32 {
        (1.0 - self.cropped_left - self.right).clamp(0.0, 1.0)
    }

    /// Vertical fraction of the sphere that is actually covered.
    pub fn vertical_fraction(&self) -> f32 {
        (1.0 - self.top - self.bottom).clamp(0.0, 1.0)
    }

    /// `true` when no bound is set (the picture covers the whole projection).
    pub fn is_empty(&self) -> bool {
        self.top == 0.0 && self.bottom == 0.0 && self.cropped_left == 0.0 && self.right == 0.0
    }
}

/// Spherical metadata of one video track.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SphericalMetadata {
    /// Stereo layout, when advertised.
    pub stereo: Option<StereoLayout>,
    /// `true` when the first half holds the right eye.
    pub swap_eyes: bool,
    /// Projection kind.
    pub projection: ProjectionKind,
    /// Crop bounds (equirectangular).  `None` when the container did not carry
    /// an `equi`/`ProjectionPrivate` element at all — an all-zero `Crop` is
    /// *metadata* and means "the frame covers the whole sphere" (360°).
    pub crop: Option<Crop>,
    /// Initial viewing pose, when advertised.
    pub pose: Option<Pose>,
    /// Which scheme the information came from.
    pub version: Option<SphericalVersion>,
    /// `true` when the coverage (360 vs 180) was derived from the picture
    /// aspect ratio because the metadata did not carry crop bounds.
    pub coverage_from_aspect: bool,
}

impl SphericalMetadata {
    /// Horizontal coverage in degrees, when it can be determined.
    pub fn coverage_degrees(&self) -> Option<f32> {
        match (self.projection, self.crop) {
            (ProjectionKind::Equirectangular, Some(crop)) => {
                Some(360.0 * crop.horizontal_fraction())
            }
            (ProjectionKind::Equirectangular, None) => None,
            _ => None,
        }
    }

    /// Resolve the [`Coverage`] enum from the metadata (and, for equirectangular
    /// sources without crop bounds, from the picture geometry).
    pub fn coverage(&self, width: u32, height: u32) -> Option<Coverage> {
        match self.projection {
            ProjectionKind::Rectangular => Some(Coverage::Planar),
            ProjectionKind::Equirectangular => {
                if let Some(crop) = self.crop {
                    // Authoritative: the bounds say how much of the sphere the
                    // frame covers (0.5 → 180°, 1.0 → 360°).
                    let frac = crop.horizontal_fraction();
                    return Some(if (frac - 0.5).abs() < 0.05 {
                        Coverage::Half180
                    } else {
                        Coverage::Full360
                    });
                }
                // No bounds.  An equirectangular picture is 360° when it is 2:1
                // and 180° when it is 1:1; for stereo layouts the *total* frame
                // geometry is what real encoders produce:
                //   mono/SBS 2:1 → 360 (SBS eyes are horizontally squeezed),
                //   mono/SBS 1:1 → 180 (VR180),
                //   TB 1:1 or wider → 360 (two 2:1 eyes stacked),
                //   TB taller than 1:1 → 180 (two 1:1 eyes stacked).
                if height == 0 {
                    return Some(Coverage::Full360);
                }
                let aspect = width as f32 / height as f32;
                let is_180 = match self.stereo {
                    Some(StereoLayout::TopBottom) => aspect < 0.9,
                    _ => aspect <= 1.15,
                };
                Some(if is_180 && aspect < 1.9 {
                    Coverage::Half180
                } else {
                    Coverage::Full360
                })
            }
            _ => None,
        }
    }
}

/// One track of the container.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TrackInfo {
    /// Track kind.
    pub kind: TrackKind,
    /// Codec id as written by the container (`avc1`, `hvc1`, `V_MKS/…`).
    pub codec: Option<String>,
    /// Coded width (video).
    pub width: u32,
    /// Coded height (video).
    pub height: u32,
    /// Display rotation in degrees, Android convention (clockwise to apply).
    pub rotation: i32,
    /// Track duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Language code (`und` when absent).
    pub language: Option<String>,
    /// Audio channel count.
    pub channels: u32,
    /// Audio sample rate in Hz.
    pub sample_rate: u32,
    /// Spherical metadata (video tracks only).
    pub spherical: Option<SphericalMetadata>,
    /// Track id inside the container.
    pub track_id: u32,
}

/// Result of probing a container.
#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    /// Container family.
    pub container: Container,
    /// Major brand / doc type (`isom`, `mp42`, `matroska`, `webm`, …).
    pub brand: Option<String>,
    /// Overall duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// All tracks found.
    pub tracks: Vec<TrackInfo>,
    /// Number of bytes inspected (diagnostics).
    pub bytes_scanned: u64,
}

impl MediaInfo {
    /// First video track, if any.
    pub fn video(&self) -> Option<&TrackInfo> {
        self.tracks.iter().find(|t| t.kind == TrackKind::Video)
    }

    /// First audio track, if any.
    pub fn audio(&self) -> Option<&TrackInfo> {
        self.tracks.iter().find(|t| t.kind == TrackKind::Audio)
    }

    /// `true` when any track carries spherical metadata.
    pub fn has_spherical(&self) -> bool {
        self.tracks.iter().any(|t| t.spherical.is_some())
    }

    /// Build the hints consumed by [`vlcrs_vr::ResolvedProjection::resolve`].
    ///
    /// Values from the platform demuxer (which are authoritative for the coded
    /// size and the rotation) can be merged on top with
    /// [`MediaInfo::hints_with`].
    pub fn hints(&self) -> MediaHints {
        let v = self.video();
        let (width, height, rotation) = match v {
            Some(t) => (t.width, t.height, t.rotation),
            None => (0, 0, 0),
        };
        let sph = v.and_then(|t| t.spherical);
        let coverage = sph.and_then(|s| s.coverage(width, height));
        let stereo = sph.and_then(|s| s.stereo);
        let swap_eyes = sph.is_some_and(|s| s.swap_eyes);
        let pose = sph.and_then(|s| s.pose);
        MediaHints {
            width,
            height,
            rotation,
            stereo,
            swap_eyes,
            coverage,
            pose,
        }
    }

    /// Same as [`MediaInfo::hints`] but override width/height/rotation with the
    /// values reported by the platform demuxer when they are known.
    pub fn hints_with(&self, width: u32, height: u32, rotation: i32) -> MediaHints {
        let mut h = self.hints();
        if width != 0 {
            h.width = width;
        }
        if height != 0 {
            h.height = height;
        }
        if rotation != 0 {
            h.rotation = rotation;
        }
        // coverage may depend on the picture geometry, recompute
        if let Some(v) = self.video() {
            if let Some(s) = v.spherical {
                h.coverage = s.coverage(h.width, h.height);
            }
        }
        h
    }

    /// One line summary for logs.
    pub fn summary(&self) -> String {
        let v = self.video();
        let a = self.audio();
        let mut s = format!(
            "{}({}) {} ms, {} track(s)",
            self.container.label(),
            self.brand.as_deref().unwrap_or("?"),
            self.duration_ms
                .map(|d| d.to_string())
                .unwrap_or_else(|| "?".into()),
            self.tracks.len()
        );
        if let Some(t) = v {
            s.push_str(&format!(
                ", video {} {}x{} rot={} sph={}",
                t.codec.as_deref().unwrap_or("?"),
                t.width,
                t.height,
                t.rotation,
                match t.spherical {
                    Some(sp) => format!(
                        "{:?}/{:?}{}{}",
                        sp.projection,
                        sp.stereo,
                        if sp.swap_eyes { "/swapped" } else { "" },
                        sp.pose
                            .map(|p| format!(" pose=({},{},{})", p.yaw, p.pitch, p.roll))
                            .unwrap_or_default()
                    ),
                    None => "none".to_string(),
                }
            ));
        }
        if let Some(t) = a {
            s.push_str(&format!(
                ", audio {} {}ch {}Hz",
                t.codec.as_deref().unwrap_or("?"),
                t.channels,
                t.sample_rate
            ));
        }
        s
    }
}

/// Probing failure.
#[derive(Clone, Debug, PartialEq)]
pub enum ProbeError {
    /// Underlying I/O error (message kept, the `std::io::Error` is not `Clone`).
    Io(String),
    /// The header does not match a supported container.
    Unsupported(String),
    /// The file ended before the metadata could be read.
    Truncated,
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::Io(m) => write!(f, "I/O error: {m}"),
            ProbeError::Unsupported(m) => write!(f, "unsupported container: {m}"),
            ProbeError::Truncated => write!(f, "truncated file"),
        }
    }
}

impl std::error::Error for ProbeError {}

impl From<std::io::Error> for ProbeError {
    fn from(e: std::io::Error) -> Self {
        ProbeError::Io(e.to_string())
    }
}

/// Default number of bytes the prober is willing to scan.
pub const DEFAULT_BUDGET: u64 = 48 * 1024 * 1024;

/// Detect the container from the first bytes.
pub fn detect_container(header: &[u8]) -> Container {
    if header.len() >= 12 && &header[4..8] == b"ftyp" {
        return Container::Mp4;
    }
    if header.len() >= 4 && header[0..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        return Container::Matroska;
    }
    // `moov`/`mdat` first (headerless streaming files)
    if header.len() >= 8 {
        let t = &header[4..8];
        if t == b"moov" || t == b"mdat" || t == b"free" || t == b"skip" || t == b"wide" {
            return Container::Mp4;
        }
    }
    Container::Unknown
}

/// Probe a container from any `Read + Seek` source.
pub fn probe<R: std::io::Read + std::io::Seek>(r: &mut R) -> Result<MediaInfo, ProbeError> {
    probe_with_budget(r, DEFAULT_BUDGET)
}

/// Probe with an explicit byte budget.
pub fn probe_with_budget<R: std::io::Read + std::io::Seek>(
    r: &mut R,
    budget: u64,
) -> Result<MediaInfo, ProbeError> {
    use std::io::SeekFrom;
    let mut header = [0u8; 12];
    let n = r.read(&mut header)?;
    if n < 4 {
        return Err(ProbeError::Truncated);
    }
    r.seek(SeekFrom::Start(0))?;
    match detect_container(&header[..n]) {
        Container::Mp4 => bmff::parse(r, budget),
        Container::Matroska => ebml::parse(r, budget),
        Container::Unknown => Err(ProbeError::Unsupported(format!(
            "magic {:02x?}",
            &header[..n.min(8)]
        ))),
    }
}

/// Probe an in-memory buffer.
pub fn probe_bytes(bytes: &[u8]) -> Result<MediaInfo, ProbeError> {
    let mut c = std::io::Cursor::new(bytes);
    probe(&mut c)
}

/// Probe a file on disk.
pub fn probe_path(path: &str) -> Result<MediaInfo, ProbeError> {
    let mut f = std::fs::File::open(path)?;
    probe(&mut f)
}
