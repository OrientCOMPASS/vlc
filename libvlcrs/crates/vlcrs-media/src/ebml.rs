//! Matroska / WebM (EBML) parser: track headers plus the `StereoMode` and
//! `Projection` elements defined by the Spherical Video V2 RFC.
//!
//! Both spellings of the projection pose found in the wild are accepted:
//! the official Matroska layout (`ProjectionPose` master `0x7673` containing
//! `0x7674`/`0x7675`/`0x7676`) and the flat layout of Google's RFC
//! (`0x7673`/`0x7674`/`0x7675` as direct children of `Projection`).

use std::io::{Read, Seek, SeekFrom};

use vlcrs_vr::StereoLayout;

use crate::{
    Container, Crop, MediaInfo, ProbeError, ProjectionKind, SphericalMetadata, SphericalVersion,
    TrackInfo, TrackKind,
};

const MAX_DEPTH: u32 = 10;
const MAX_ELEMENTS: u32 = 400_000;
const MAX_BINARY: u64 = 4096;

// element ids
const ID_EBML: u64 = 0x1A45_DFA3;
const ID_DOCTYPE: u64 = 0x4282;
const ID_SEGMENT: u64 = 0x1853_8067;
const ID_INFO: u64 = 0x1549_A966;
const ID_TIMECODE_SCALE: u64 = 0x2A_D7_B1;
const ID_DURATION: u64 = 0x4489;
const ID_TRACKS: u64 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u64 = 0xAE;
const ID_TRACK_NUMBER: u64 = 0xD7;
const ID_TRACK_TYPE: u64 = 0x83;
const ID_CODEC_ID: u64 = 0x86;
const ID_LANGUAGE: u64 = 0x22_B59C;
const ID_VIDEO: u64 = 0xE0;
const ID_AUDIO: u64 = 0xE1;
const ID_PIXEL_WIDTH: u64 = 0xB0;
const ID_PIXEL_HEIGHT: u64 = 0xBA;
const ID_DISPLAY_WIDTH: u64 = 0x54_B0;
const ID_DISPLAY_HEIGHT: u64 = 0x54_BA;
const ID_STEREO_MODE: u64 = 0x53_B8;
const ID_CHANNELS: u64 = 0x9F;
const ID_SAMPLING_FREQ: u64 = 0xB5;
const ID_PROJECTION: u64 = 0x7670;
const ID_PROJECTION_TYPE: u64 = 0x7671;
const ID_PROJECTION_PRIVATE: u64 = 0x7672;
const ID_PROJECTION_POSE: u64 = 0x7673;
const ID_POSE_YAW: u64 = 0x7674;
const ID_POSE_PITCH: u64 = 0x7675;
const ID_POSE_ROLL: u64 = 0x7676;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    Root,
    Ebml,
    Segment,
    Info,
    Tracks,
    TrackEntry,
    Video,
    Audio,
    Projection,
    /// Inside the official `ProjectionPose` master.
    Pose,
}

struct Parse {
    info: MediaInfo,
    track: TrackInfo,
    timecode_scale: f64,
    display_wh: (u32, u32),
    budget_left: u64,
    budget_total: u64,
    elements: u32,
    saw_tracks: bool,
    saw_info: bool,
}

/// Parse a Matroska/WebM stream.
pub fn parse<R: Read + Seek>(r: &mut R, budget: u64) -> Result<MediaInfo, ProbeError> {
    let end = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;
    let mut p = Parse {
        info: MediaInfo {
            container: Container::Matroska,
            ..Default::default()
        },
        track: TrackInfo::default(),
        timecode_scale: 1_000_000.0,
        display_wh: (0, 0),
        budget_left: budget,
        budget_total: budget,
        elements: 0,
        saw_tracks: false,
        saw_info: false,
    };
    walk(r, &mut p, 0, end, 0, Ctx::Root)?;
    p.info.bytes_scanned = budget.saturating_sub(p.budget_left);
    Ok(p.info)
}

/// Read a variable size integer.
///
/// Returns `(value, length, unknown_size)`; `masked` controls whether the VINT
/// marker bit is removed (data sizes) or kept (element ids).
fn read_vint<R: Read>(
    r: &mut R,
    max_len: usize,
    masked: bool,
    p: &mut Parse,
) -> Result<Option<(u64, usize, bool)>, ProbeError> {
    let mut first = [0u8; 1];
    if p.budget_left < 1 {
        return Err(ProbeError::Truncated);
    }
    if r.read(&mut first)? == 0 {
        return Ok(None);
    }
    p.budget_left -= 1;
    let lead = first[0].leading_zeros() as usize;
    let len = lead + 1;
    if len > max_len || len > 8 {
        return Ok(None);
    }
    // The VINT marker bit is part of an element id but not of a data size.
    let value_mask: u8 = if !masked {
        0xff
    } else if lead >= 7 {
        // 8 byte wide size: the first byte only carries the marker
        0
    } else {
        ((0xffu16 >> (lead + 1)) & 0xff) as u8
    };
    let mut value = u64::from(first[0] & value_mask);
    let mut all_ones = masked && (first[0] & value_mask) == value_mask;
    if len > 1 {
        let mut rest = vec![0u8; len - 1];
        if p.budget_left < (len - 1) as u64 {
            return Err(ProbeError::Truncated);
        }
        r.read_exact(&mut rest)?;
        p.budget_left -= (len - 1) as u64;
        for b in &rest {
            value = (value << 8) | u64::from(*b);
            if *b != 0xff {
                all_ones = false;
            }
        }
    }
    Ok(Some((value, len, masked && all_ones)))
}

fn walk<R: Read + Seek>(
    r: &mut R,
    p: &mut Parse,
    start: u64,
    end: u64,
    depth: u32,
    ctx: Ctx,
) -> Result<(), ProbeError> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    r.seek(SeekFrom::Start(start))?;
    let mut pos = start;
    while pos < end {
        p.elements += 1;
        if p.elements > MAX_ELEMENTS || p.budget_left < 2 {
            return Ok(());
        }
        let id = match read_vint(r, 4, false, p)? {
            Some((v, len, _)) => (v, len),
            None => return Ok(()),
        };
        let size = match read_vint(r, 8, true, p)? {
            Some((v, len, unknown)) => (v, len, unknown),
            None => return Ok(()),
        };
        let header_len = (id.1 + size.1) as u64;
        let data_start = pos + header_len;
        let data_len = if size.2 {
            // unknown size: runs to the end of the parent
            end.saturating_sub(data_start)
        } else {
            size.0
        };
        let data_end = data_start.saturating_add(data_len).min(end);
        if data_end < data_start {
            return Ok(());
        }
        handle(r, p, id.0, data_start, data_end, depth, ctx)?;
        if size.2 {
            // unknown size master: the walker consumed everything
            return Ok(());
        }
        // data_start already includes the header, and the header is at least 2
        // bytes long, so `pos` always advances.
        pos = data_start + data_len;
        r.seek(SeekFrom::Start(pos))?;
        // Stop early once everything interesting has been seen: `Tracks` and
        // `Info` normally sit at the very beginning of the segment, well before
        // the clusters.
        if ctx == Ctx::Segment
            && p.saw_tracks
            && (p.saw_info || p.budget_total.saturating_sub(p.budget_left) > 8 * 1024 * 1024)
        {
            return Ok(());
        }
    }
    Ok(())
}

fn read_bytes<R: Read + Seek>(
    r: &mut R,
    p: &mut Parse,
    start: u64,
    end: u64,
    cap: u64,
) -> Result<Vec<u8>, ProbeError> {
    let want = end.saturating_sub(start).min(cap);
    if p.budget_left < want {
        return Err(ProbeError::Truncated);
    }
    let mut v = vec![0u8; want as usize];
    if want > 0 {
        r.seek(SeekFrom::Start(start))?;
        r.read_exact(&mut v)?;
        p.budget_left -= want;
    }
    Ok(v)
}

fn uint(b: &[u8]) -> u64 {
    let mut v = 0u64;
    for x in b {
        v = (v << 8) | u64::from(*x);
    }
    v
}

fn float(b: &[u8]) -> f64 {
    match b.len() {
        4 => f32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64,
        8 => f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
        _ => 0.0,
    }
}

fn string(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

fn ensure_spherical(t: &mut TrackInfo) -> &mut SphericalMetadata {
    if t.spherical.is_none() {
        t.spherical = Some(SphericalMetadata {
            projection: ProjectionKind::Unknown,
            ..Default::default()
        });
    }
    t.spherical.as_mut().expect("just set")
}

/// Matroska `StereoMode` → layout + eye order.
pub fn stereo_mode(v: u64) -> Option<(StereoLayout, bool)> {
    match v {
        0 => Some((StereoLayout::Mono, false)),
        1 => Some((StereoLayout::SideBySide, false)), // side by side, left first
        2 => Some((StereoLayout::TopBottom, true)),   // top-bottom, right first
        3 => Some((StereoLayout::TopBottom, false)),  // top-bottom, left first
        11 => Some((StereoLayout::SideBySide, true)), // side by side, right first
        _ => None, // anaglyph / interleaved / frame interleaved: unsupported
    }
}

fn handle<R: Read + Seek>(
    r: &mut R,
    p: &mut Parse,
    id: u64,
    start: u64,
    end: u64,
    depth: u32,
    ctx: Ctx,
) -> Result<(), ProbeError> {
    match (id, ctx) {
        (ID_EBML, Ctx::Root) => walk(r, p, start, end, depth + 1, Ctx::Ebml)?,
        (ID_DOCTYPE, Ctx::Ebml) => {
            let v = read_bytes(r, p, start, end, 64)?;
            let s = string(&v);
            if !s.is_empty() {
                p.info.brand = Some(s);
            }
        }
        (ID_SEGMENT, Ctx::Root) => walk(r, p, start, end, depth + 1, Ctx::Segment)?,
        (ID_INFO, Ctx::Segment) => {
            p.saw_info = true;
            walk(r, p, start, end, depth + 1, Ctx::Info)?;
        }
        (ID_TIMECODE_SCALE, Ctx::Info) => {
            let v = read_bytes(r, p, start, end, 8)?;
            let scale = uint(&v);
            if scale > 0 {
                p.timecode_scale = scale as f64;
            }
        }
        (ID_DURATION, Ctx::Info) => {
            let v = read_bytes(r, p, start, end, 8)?;
            let d = float(&v);
            if d.is_finite() && d > 0.0 && d < 1e15 {
                p.info.duration_ms = Some((d * p.timecode_scale / 1_000_000.0) as u64);
            }
        }
        (ID_TRACKS, Ctx::Segment) => {
            p.saw_tracks = true;
            walk(r, p, start, end, depth + 1, Ctx::Tracks)?;
        }
        (ID_TRACK_ENTRY, Ctx::Tracks) => {
            p.track = TrackInfo::default();
            p.display_wh = (0, 0);
            walk(r, p, start, end, depth + 1, Ctx::TrackEntry)?;
            if p.track.kind != TrackKind::Other {
                p.info.tracks.push(std::mem::take(&mut p.track));
            }
        }
        (ID_TRACK_NUMBER, Ctx::TrackEntry) => {
            let v = read_bytes(r, p, start, end, 8)?;
            p.track.track_id = uint(&v) as u32;
        }
        (ID_TRACK_TYPE, Ctx::TrackEntry) => {
            let v = read_bytes(r, p, start, end, 8)?;
            p.track.kind = match uint(&v) {
                1 => TrackKind::Video,
                2 => TrackKind::Audio,
                0x11 | 0x12 => TrackKind::Subtitle,
                _ => TrackKind::Other,
            };
        }
        (ID_CODEC_ID, Ctx::TrackEntry) => {
            let v = read_bytes(r, p, start, end, 64)?;
            let s = string(&v);
            if !s.is_empty() {
                p.track.codec = Some(s);
            }
        }
        (ID_LANGUAGE, Ctx::TrackEntry) => {
            let v = read_bytes(r, p, start, end, 16)?;
            let s = string(&v);
            if !s.is_empty() && s != "und" {
                p.track.language = Some(s);
            }
        }
        (ID_VIDEO, Ctx::TrackEntry) => walk(r, p, start, end, depth + 1, Ctx::Video)?,
        (ID_AUDIO, Ctx::TrackEntry) => walk(r, p, start, end, depth + 1, Ctx::Audio)?,
        (ID_PIXEL_WIDTH, Ctx::Video) => {
            let v = read_bytes(r, p, start, end, 8)?;
            p.track.width = uint(&v) as u32;
        }
        (ID_PIXEL_HEIGHT, Ctx::Video) => {
            let v = read_bytes(r, p, start, end, 8)?;
            p.track.height = uint(&v) as u32;
        }
        (ID_DISPLAY_WIDTH, Ctx::Video) => {
            let v = read_bytes(r, p, start, end, 8)?;
            p.display_wh.0 = uint(&v) as u32;
        }
        (ID_DISPLAY_HEIGHT, Ctx::Video) => {
            let v = read_bytes(r, p, start, end, 8)?;
            p.display_wh.1 = uint(&v) as u32;
        }
        (ID_STEREO_MODE, Ctx::Video) => {
            let v = read_bytes(r, p, start, end, 8)?;
            if let Some((layout, swap)) = stereo_mode(uint(&v)) {
                let s = ensure_spherical(&mut p.track);
                if s.stereo.is_none() {
                    s.stereo = Some(layout);
                    s.swap_eyes = swap;
                }
                s.version = Some(SphericalVersion::Matroska);
            }
        }
        (ID_CHANNELS, Ctx::Audio) => {
            let v = read_bytes(r, p, start, end, 8)?;
            p.track.channels = uint(&v) as u32;
        }
        (ID_SAMPLING_FREQ, Ctx::Audio) => {
            let v = read_bytes(r, p, start, end, 8)?;
            p.track.sample_rate = float(&v) as u32;
        }
        (ID_PROJECTION, Ctx::Video) => {
            let s = ensure_spherical(&mut p.track);
            s.version = Some(SphericalVersion::Matroska);
            walk(r, p, start, end, depth + 1, Ctx::Projection)?;
        }
        (ID_PROJECTION_TYPE, Ctx::Projection) => {
            let v = read_bytes(r, p, start, end, 8)?;
            let s = ensure_spherical(&mut p.track);
            s.projection = match uint(&v) {
                0 => ProjectionKind::Rectangular,
                1 => ProjectionKind::Equirectangular,
                2 => ProjectionKind::Cubemap,
                3 => ProjectionKind::Mesh,
                _ => ProjectionKind::Unknown,
            };
        }
        (ID_PROJECTION_PRIVATE, Ctx::Projection) => {
            let v = read_bytes(r, p, start, end, MAX_BINARY)?;
            // Same payload as an ISOBFF `equi`/`cbmp` box body: FullBox
            // version+flags followed by four 0.32 bounds (equirectangular).
            if v.len() >= 20 {
                let s = ensure_spherical(&mut p.track);
                let f = |i: usize| -> f32 {
                    let raw = u32::from_be_bytes([v[i], v[i + 1], v[i + 2], v[i + 3]]);
                    (raw as f64 / (1u64 << 32) as f64) as f32
                };
                s.crop = Some(Crop {
                    top: f(4),
                    bottom: f(8),
                    cropped_left: f(12),
                    right: f(16),
                });
            }
        }
        // Official layout: ProjectionPose master holding yaw/pitch/roll
        (ID_PROJECTION_POSE, Ctx::Projection) => {
            let len = end.saturating_sub(start);
            if len == 4 || len == 8 {
                // Google's flat layout: 0x7673 is the yaw float itself
                let v = read_bytes(r, p, start, end, 8)?;
                let s = ensure_spherical(&mut p.track);
                let mut pose = s.pose.unwrap_or_default();
                pose.yaw = float(&v) as f32;
                s.pose = Some(pose);
            } else {
                walk(r, p, start, end, depth + 1, Ctx::Pose)?;
            }
        }
        (ID_POSE_YAW, Ctx::Pose) => set_pose(r, p, start, end, 0)?,
        (ID_POSE_PITCH, Ctx::Pose) => set_pose(r, p, start, end, 1)?,
        (ID_POSE_ROLL, Ctx::Pose) => set_pose(r, p, start, end, 2)?,
        // Google's flat layout at the Projection level
        (ID_POSE_YAW, Ctx::Projection) => set_pose(r, p, start, end, 1)?,
        (ID_POSE_PITCH, Ctx::Projection) => set_pose(r, p, start, end, 2)?,
        _ => {}
    }
    Ok(())
}

fn set_pose<R: Read + Seek>(
    r: &mut R,
    p: &mut Parse,
    start: u64,
    end: u64,
    which: u8,
) -> Result<(), ProbeError> {
    let v = read_bytes(r, p, start, end, 8)?;
    let s = ensure_spherical(&mut p.track);
    let mut pose = s.pose.unwrap_or_default();
    let value = float(&v) as f32;
    match which {
        0 => pose.yaw = value,
        1 => pose.pitch = value,
        _ => pose.roll = value,
    }
    s.pose = Some(pose);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect_container;
    use vlcrs_vr::Coverage;

    /// Minimal EBML writer.
    struct Mkv(Vec<u8>);

    impl Mkv {
        fn new() -> Self {
            Mkv(Vec::new())
        }
        fn write_id(&mut self, id: u64) {
            // ids are stored verbatim, big endian, minimal length
            let bytes = id.to_be_bytes();
            let first_non_zero = bytes
                .iter()
                .position(|b| *b != 0)
                .unwrap_or(bytes.len() - 1);
            self.0.extend_from_slice(&bytes[first_non_zero..]);
        }
        fn write_size(&mut self, size: u64) {
            // pick the smallest vint width that fits
            for len in 1..8usize {
                let max = (1u64 << (7 * len)) - 2;
                if size <= max {
                    let marker = 1u64 << (7 * len);
                    let v = marker | size;
                    let bytes = v.to_be_bytes();
                    let start = bytes.len() - len;
                    self.0.extend_from_slice(&bytes[start..]);
                    return;
                }
            }
            panic!("size too large");
        }
        /// Reserve a width-8 size field (always valid EBML) and return its offset.
        fn master_begin(&mut self, id: u64) -> usize {
            self.write_id(id);
            let at = self.0.len();
            self.0.extend_from_slice(&[0x01, 0, 0, 0, 0, 0, 0, 0]);
            at
        }
        fn master_end(&mut self, at: usize) {
            let size = (self.0.len() - at - 8) as u64;
            assert!(size < (1u64 << 56), "master too large for the test writer");
            let v = (1u64 << 56) | size;
            self.0[at..at + 8].copy_from_slice(&v.to_be_bytes());
        }
        fn uint_elem(&mut self, id: u64, v: u64) {
            let bytes = v.to_be_bytes();
            let first = bytes
                .iter()
                .position(|b| *b != 0)
                .unwrap_or(bytes.len() - 1);
            let body = &bytes[first..];
            self.write_id(id);
            self.write_size(body.len() as u64);
            self.0.extend_from_slice(body);
        }
        fn float_elem(&mut self, id: u64, v: f64) {
            let body = v.to_be_bytes();
            self.write_id(id);
            self.write_size(body.len() as u64);
            self.0.extend_from_slice(&body);
        }
        fn str_elem(&mut self, id: u64, s: &str) {
            self.write_id(id);
            self.write_size(s.len() as u64);
            self.0.extend_from_slice(s.as_bytes());
        }
        fn bin_elem(&mut self, id: u64, b: &[u8]) {
            self.write_id(id);
            self.write_size(b.len() as u64);
            self.0.extend_from_slice(b);
        }
    }

    fn equi_body(bounds: [f32; 4]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&0u32.to_be_bytes()); // version + flags
        for b in bounds {
            let raw = (f64::from(b) * (1u64 << 32) as f64) as u32;
            v.extend_from_slice(&raw.to_be_bytes());
        }
        v
    }

    /// Build a WebM-like file: EBML header + Segment(Info + Tracks(video+audio)).
    fn build(video: impl FnOnce(&mut Mkv), audio: bool) -> Vec<u8> {
        let mut f = Mkv::new();
        let ebml = f.master_begin(ID_EBML);
        f.uint_elem(0x4286, 1); // EBMLVersion
        f.str_elem(ID_DOCTYPE, "webm");
        f.master_end(ebml);
        let seg = f.master_begin(ID_SEGMENT);
        let info = f.master_begin(ID_INFO);
        f.uint_elem(ID_TIMECODE_SCALE, 1_000_000);
        // Duration is expressed in timecode units: 12500 × 1 ms = 12.5 s
        f.float_elem(ID_DURATION, 12_500.0);
        f.master_end(info);
        let tracks = f.master_begin(ID_TRACKS);
        let te = f.master_begin(ID_TRACK_ENTRY);
        f.uint_elem(ID_TRACK_NUMBER, 1);
        f.uint_elem(ID_TRACK_TYPE, 1);
        f.str_elem(ID_CODEC_ID, "V_VP9");
        let vid = f.master_begin(ID_VIDEO);
        video(&mut f);
        f.master_end(vid);
        f.master_end(te);
        if audio {
            let te = f.master_begin(ID_TRACK_ENTRY);
            f.uint_elem(ID_TRACK_NUMBER, 2);
            f.uint_elem(ID_TRACK_TYPE, 2);
            f.str_elem(ID_CODEC_ID, "A_OPUS");
            let aud = f.master_begin(ID_AUDIO);
            f.uint_elem(ID_CHANNELS, 6);
            f.float_elem(ID_SAMPLING_FREQ, 48_000.0);
            f.master_end(aud);
            f.master_end(te);
        }
        f.master_end(tracks);
        f.master_end(seg);
        f.0
    }

    fn probe(bytes: &[u8]) -> MediaInfo {
        crate::probe_bytes(bytes).expect("probe failed")
    }

    #[test]
    fn detects_matroska_and_basic_info() {
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 3840);
                f.uint_elem(ID_PIXEL_HEIGHT, 1920);
            },
            true,
        );
        assert_eq!(detect_container(&bytes), Container::Matroska);
        let info = probe(&bytes);
        assert_eq!(info.container, Container::Matroska);
        assert_eq!(info.brand.as_deref(), Some("webm"));
        assert_eq!(info.duration_ms, Some(12_500));
        assert_eq!(info.tracks.len(), 2);
        let v = info.video().unwrap();
        assert_eq!(v.codec.as_deref(), Some("V_VP9"));
        assert_eq!((v.width, v.height), (3840, 1920));
        let a = info.audio().unwrap();
        assert_eq!(a.channels, 6);
        assert_eq!(a.sample_rate, 48_000);
        assert_eq!(a.codec.as_deref(), Some("A_OPUS"));
    }

    #[test]
    fn stereo_mode_mapping() {
        assert_eq!(stereo_mode(0), Some((StereoLayout::Mono, false)));
        assert_eq!(stereo_mode(1), Some((StereoLayout::SideBySide, false)));
        assert_eq!(stereo_mode(3), Some((StereoLayout::TopBottom, false)));
        assert_eq!(stereo_mode(2), Some((StereoLayout::TopBottom, true)));
        assert_eq!(stereo_mode(11), Some((StereoLayout::SideBySide, true)));
        assert_eq!(stereo_mode(10), None); // anaglyph: unsupported
    }

    #[test]
    fn reads_stereo_mode_element() {
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 3840);
                f.uint_elem(ID_PIXEL_HEIGHT, 1920);
                f.uint_elem(ID_STEREO_MODE, 1);
            },
            false,
        );
        let info = probe(&bytes);
        let s = info.video().unwrap().spherical.unwrap();
        assert_eq!(s.stereo, Some(StereoLayout::SideBySide));
        assert!(!s.swap_eyes);
        assert_eq!(s.version, Some(SphericalVersion::Matroska));
    }

    #[test]
    fn reads_projection_official_layout() {
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 1920);
                f.uint_elem(ID_PIXEL_HEIGHT, 1920);
                let proj = f.master_begin(ID_PROJECTION);
                f.uint_elem(ID_PROJECTION_TYPE, 1);
                let pose = f.master_begin(ID_PROJECTION_POSE);
                f.float_elem(ID_POSE_YAW, 45.0);
                f.float_elem(ID_POSE_PITCH, -15.0);
                f.float_elem(ID_POSE_ROLL, 5.0);
                f.master_end(pose);
                f.master_end(proj);
            },
            false,
        );
        let info = probe(&bytes);
        let s = info.video().unwrap().spherical.unwrap();
        assert_eq!(s.projection, ProjectionKind::Equirectangular);
        let pose = s.pose.unwrap();
        assert!((pose.yaw - 45.0).abs() < 1e-3, "{pose:?}");
        assert!((pose.pitch + 15.0).abs() < 1e-3, "{pose:?}");
        assert!((pose.roll - 5.0).abs() < 1e-3, "{pose:?}");
        // 1:1 equirect without bounds → 180°
        assert_eq!(s.coverage(1920, 1920), Some(Coverage::Half180));
    }

    #[test]
    fn reads_projection_google_flat_layout() {
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 3840);
                f.uint_elem(ID_PIXEL_HEIGHT, 1920);
                let proj = f.master_begin(ID_PROJECTION);
                f.uint_elem(ID_PROJECTION_TYPE, 1);
                // 0x7673 as a float leaf = yaw, 0x7674 = pitch, 0x7675 = roll
                f.float_elem(ID_PROJECTION_POSE, 30.0);
                f.float_elem(ID_POSE_YAW, -10.0);
                f.float_elem(ID_POSE_PITCH, 3.0);
                f.master_end(proj);
            },
            false,
        );
        let s = probe(&bytes).video().unwrap().spherical.unwrap();
        let pose = s.pose.unwrap();
        assert!((pose.yaw - 30.0).abs() < 1e-3, "{pose:?}");
        assert!((pose.pitch + 10.0).abs() < 1e-3, "{pose:?}");
        assert!((pose.roll - 3.0).abs() < 1e-3, "{pose:?}");
    }

    #[test]
    fn reads_projection_private_bounds() {
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 1920);
                f.uint_elem(ID_PIXEL_HEIGHT, 1920);
                let proj = f.master_begin(ID_PROJECTION);
                f.uint_elem(ID_PROJECTION_TYPE, 1);
                f.bin_elem(ID_PROJECTION_PRIVATE, &equi_body([0.0, 0.0, 0.25, 0.25]));
                f.master_end(proj);
            },
            false,
        );
        let s = probe(&bytes).video().unwrap().spherical.unwrap();
        let crop = s.crop.expect("ProjectionPrivate bounds");
        assert!((crop.cropped_left - 0.25).abs() < 1e-6);
        assert!((crop.right - 0.25).abs() < 1e-6);
        assert!((s.coverage_degrees().unwrap() - 180.0).abs() < 0.01);
        assert_eq!(s.coverage(1920, 1920), Some(Coverage::Half180));
    }

    #[test]
    fn cubemap_and_mesh_are_reported_unsupported() {
        for (ty, expected) in [(2u64, ProjectionKind::Cubemap), (3, ProjectionKind::Mesh)] {
            let bytes = build(
                |f| {
                    f.uint_elem(ID_PIXEL_WIDTH, 1920);
                    f.uint_elem(ID_PIXEL_HEIGHT, 1080);
                    let proj = f.master_begin(ID_PROJECTION);
                    f.uint_elem(ID_PROJECTION_TYPE, ty);
                    f.master_end(proj);
                },
                false,
            );
            let s = probe(&bytes).video().unwrap().spherical.unwrap();
            assert_eq!(s.projection, expected);
            assert!(!s.projection.supported());
        }
    }

    #[test]
    fn rectangular_projection_is_planar() {
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 1920);
                f.uint_elem(ID_PIXEL_HEIGHT, 1080);
                let proj = f.master_begin(ID_PROJECTION);
                f.uint_elem(ID_PROJECTION_TYPE, 0);
                f.master_end(proj);
            },
            false,
        );
        let s = probe(&bytes).video().unwrap().spherical.unwrap();
        assert_eq!(s.projection, ProjectionKind::Rectangular);
        assert_eq!(s.coverage(1920, 1080), Some(Coverage::Planar));
    }

    #[test]
    fn sbs_180_webm_is_detected() {
        // A 180° SBS WebM carries ProjectionPrivate with 25% crops on both
        // sides; without them a 2:1 SBS frame is ambiguous and resolves to 360°.
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 3840);
                f.uint_elem(ID_PIXEL_HEIGHT, 1920);
                f.uint_elem(ID_STEREO_MODE, 1);
                let proj = f.master_begin(ID_PROJECTION);
                f.uint_elem(ID_PROJECTION_TYPE, 1);
                f.bin_elem(ID_PROJECTION_PRIVATE, &equi_body([0.0, 0.0, 0.25, 0.25]));
                f.master_end(proj);
            },
            false,
        );
        let info = probe(&bytes);
        let h = info.hints();
        assert_eq!(h.stereo, Some(StereoLayout::SideBySide));
        assert_eq!(h.coverage, Some(Coverage::Half180));
        assert!(h.has_metadata());

        // …and the ambiguous variant falls back to the geometry rule (360°)
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 3840);
                f.uint_elem(ID_PIXEL_HEIGHT, 1920);
                f.uint_elem(ID_STEREO_MODE, 1);
                let proj = f.master_begin(ID_PROJECTION);
                f.uint_elem(ID_PROJECTION_TYPE, 1);
                f.master_end(proj);
            },
            false,
        );
        assert_eq!(
            probe(&bytes).hints().coverage,
            Some(Coverage::Full360),
            "2:1 SBS without bounds is the squeezed-eyes 360 convention"
        );
    }

    #[test]
    fn hints_without_projection_are_empty() {
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 1920);
                f.uint_elem(ID_PIXEL_HEIGHT, 1080);
            },
            true,
        );
        let h = probe(&bytes).hints();
        assert!(!h.has_metadata());
        assert_eq!(h.coverage, None);
        assert_eq!((h.width, h.height), (1920, 1080));
    }

    #[test]
    fn truncated_files_do_not_panic() {
        let bytes = build(
            |f| {
                f.uint_elem(ID_PIXEL_WIDTH, 1920);
                f.uint_elem(ID_PIXEL_HEIGHT, 1080);
            },
            true,
        );
        for cut in [0usize, 3, 4, 20, 64, 200, bytes.len() / 2, bytes.len() - 1] {
            let mut c = std::io::Cursor::new(&bytes[..cut.min(bytes.len())]);
            let _ = crate::probe(&mut c);
        }
    }

    #[test]
    fn vint_roundtrip() {
        // sizes written by the test writer must be readable
        let mut f = Mkv::new();
        for size in [0u64, 1, 126, 127, 128, 16_382, 16_383, 16_384, 2_097_150] {
            let before = f.0.len();
            f.write_size(size);
            let written = f.0[before..].to_vec();
            let mut c = std::io::Cursor::new(written);
            let mut p = Parse {
                info: MediaInfo::default(),
                track: TrackInfo::default(),
                timecode_scale: 1.0,
                display_wh: (0, 0),
                budget_left: 1024,
                budget_total: 1024,
                elements: 0,
                saw_tracks: false,
                saw_info: false,
            };
            let got = read_vint(&mut c, 8, true, &mut p).unwrap().expect("vint");
            assert_eq!(got.0, size, "size {size} → {:?}", got);
        }
    }
}
