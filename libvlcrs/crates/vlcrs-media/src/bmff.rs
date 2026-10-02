//! ISO-BMFF (MP4/MOV) parser: just enough structure to reach the track headers
//! and the Spherical Video V1/V2 metadata.

use std::io::{Read, Seek, SeekFrom};

use vlcrs_vr::{Pose, StereoLayout};

use crate::{
    Container, Crop, MediaInfo, ProbeError, ProjectionKind, SphericalMetadata, SphericalVersion,
    TrackInfo, TrackKind,
};

/// Maximum nesting depth accepted.
const MAX_DEPTH: u32 = 12;
/// Maximum number of boxes walked (guards against pathological files).
const MAX_BOXES: u32 = 400_000;
/// Largest leaf payload read into memory.
const MAX_LEAF: u64 = 256 * 1024;

/// Spherical Video V1 UUID: `ffcc8263-f855-4a93-8814-587a02521fdd`.
pub const SPHERICAL_V1_UUID: [u8; 16] = [
    0xff, 0xcc, 0x82, 0x63, 0xf8, 0x55, 0x4a, 0x93, 0x88, 0x14, 0x58, 0x7a, 0x02, 0x52, 0x1f, 0xdd,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ctx {
    Top,
    Moov,
    Trak,
    Mdia,
    Minf,
    Stbl,
    SampleEntry,
    Sv3d,
    Proj,
}

struct Parse {
    info: MediaInfo,
    track: TrackInfo,
    track_timescale: u32,
    mvhd_timescale: u32,
    in_video_entry: bool,
    budget_left: u64,
    boxes: u32,
}

/// Parse an ISO-BMFF stream.
pub fn parse<R: Read + Seek>(r: &mut R, budget: u64) -> Result<MediaInfo, ProbeError> {
    let end = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;
    let mut p = Parse {
        info: MediaInfo {
            container: Container::Mp4,
            ..Default::default()
        },
        track: TrackInfo::default(),
        track_timescale: 0,
        mvhd_timescale: 0,
        in_video_entry: false,
        budget_left: budget,
        boxes: 0,
    };
    walk(r, &mut p, 0, end, 0, Ctx::Top)?;
    p.info.bytes_scanned = budget.saturating_sub(p.budget_left);
    Ok(p.info)
}

fn walk<R: Read + Seek>(
    r: &mut R,
    p: &mut Parse,
    start: u64,
    end: u64,
    depth: u32,
    ctx: Ctx,
) -> Result<(), ProbeError> {
    if depth > MAX_DEPTH || start >= end {
        return Ok(());
    }
    let mut pos = start;
    while pos + 8 <= end {
        p.boxes += 1;
        if p.boxes > MAX_BOXES || p.budget_left < 8 {
            return Ok(());
        }
        let mut hdr = [0u8; 8];
        if read_at(r, pos, &mut hdr, p).is_err() {
            return Ok(());
        }
        let mut size = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
        let fourcc = [hdr[4], hdr[5], hdr[6], hdr[7]];
        let mut payload = pos + 8;
        if size == 1 {
            let mut large = [0u8; 8];
            if read_at(r, pos + 8, &mut large, p).is_err() {
                return Ok(());
            }
            size = u64::from_be_bytes(large);
            payload = pos + 16;
        } else if size == 0 {
            // box extends to the end of the enclosing range
            size = end - pos;
        }
        if size < 8 || pos + size > end {
            // truncated / inconsistent: stop walking this level
            return Ok(());
        }
        let box_end = pos + size;
        handle(r, p, fourcc, payload, box_end, depth, ctx)?;
        pos = box_end;
    }
    Ok(())
}

fn read_at<R: Read + Seek>(
    r: &mut R,
    pos: u64,
    buf: &mut [u8],
    p: &mut Parse,
) -> Result<(), ProbeError> {
    let len = buf.len() as u64;
    if p.budget_left < len {
        return Err(ProbeError::Truncated);
    }
    r.seek(SeekFrom::Start(pos))?;
    r.read_exact(buf)?;
    p.budget_left = p.budget_left.saturating_sub(len);
    Ok(())
}

/// Read up to `cap` bytes of a leaf box.
fn leaf<R: Read + Seek>(
    r: &mut R,
    p: &mut Parse,
    payload: u64,
    end: u64,
    cap: u64,
) -> Result<Vec<u8>, ProbeError> {
    let want = (end.saturating_sub(payload)).min(cap).min(MAX_LEAF);
    let mut v = vec![0u8; want as usize];
    if want > 0 {
        read_at(r, payload, &mut v, p)?;
    }
    Ok(v)
}

fn u16be(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}

fn u32be(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn i32be(b: &[u8], i: usize) -> i32 {
    i32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// 16.16 fixed point → float.
fn fixed_16_16(v: i32) -> f32 {
    v as f32 / 65536.0
}

/// `equi` projection bounds: 0.32 fixed point fraction.
fn fixed_0_32(v: u32) -> f32 {
    (v as f64 / (1u64 << 32) as f64) as f32
}

fn fourcc_str(b: [u8; 4]) -> String {
    b.iter()
        .map(|c| {
            if c.is_ascii_graphic() || *c == b' ' {
                *c as char
            } else {
                '.'
            }
        })
        .collect()
}

fn unpack_language(v: u16) -> String {
    let c = |shift: u32| -> char {
        let code = ((v >> shift) & 0x1f) as u8;
        if code == 0 {
            b'?' as char
        } else {
            (b'a' - 1 + code) as char
        }
    };
    let s = format!("{}{}{}", c(10), c(5), c(0));
    if s == "und" || s.contains('?') {
        String::new()
    } else {
        s
    }
}

fn is_video_entry(f: &[u8; 4]) -> bool {
    matches!(
        f,
        b"avc1"
            | b"avc3"
            | b"avc4"
            | b"hvc1"
            | b"hev1"
            | b"hvt1"
            | b"hev2"
            | b"mp4v"
            | b"vp08"
            | b"vp09"
            | b"av01"
            | b"av1x"
            | b"mvc1"
            | b"mvc2"
            | b"dvav"
            | b"dva1"
            | b"dvhe"
            | b"dvh1"
            | b"drac"
            | b"encv"
    )
}

fn is_audio_entry(f: &[u8; 4]) -> bool {
    matches!(
        f,
        b"mp4a"
            | b"ac-3"
            | b"ec-3"
            | b"ac-4"
            | b"Opus"
            | b"opus"
            | b"fLaC"
            | b"alac"
            | b"samr"
            | b"sawb"
            | b"sawp"
            | b"twos"
            | b"sowt"
            | b"ulaw"
            | b"alaw"
            | b".mp3"
            | b"dtsc"
            | b"dtsh"
            | b"dtsl"
            | b"dtse"
            | b"enca"
            | b"flac"
            | b"mha1"
            | b"mha2"
            | b"mhm1"
            | b"mhm2"
    )
}

#[allow(clippy::too_many_arguments)]
fn handle<R: Read + Seek>(
    r: &mut R,
    p: &mut Parse,
    fourcc: [u8; 4],
    payload: u64,
    end: u64,
    depth: u32,
    ctx: Ctx,
) -> Result<(), ProbeError> {
    match (&fourcc, ctx) {
        (b"ftyp", Ctx::Top) => {
            let v = leaf(r, p, payload, end, 512)?;
            if v.len() >= 4 {
                p.info.brand = Some(fourcc_str([v[0], v[1], v[2], v[3]]));
            }
        }
        (b"moov", Ctx::Top) => walk(r, p, payload, end, depth + 1, Ctx::Moov)?,
        (b"mvhd", Ctx::Moov) => {
            let v = leaf(r, p, payload, end, 128)?;
            if v.len() >= 20 {
                let version = v[0];
                let (ts, dur) = if version == 1 {
                    if v.len() < 32 {
                        return Ok(());
                    }
                    (
                        u32be(&v, 20),
                        ((u64::from_be_bytes([
                            v[24], v[25], v[26], v[27], v[28], v[29], v[30], v[31],
                        ])) as f64),
                    )
                } else {
                    (u32be(&v, 12), u32be(&v, 16) as f64)
                };
                p.mvhd_timescale = ts;
                if ts > 0 && dur > 0.0 && dur < (u64::MAX / 2) as f64 {
                    p.info.duration_ms = Some((dur * 1000.0 / f64::from(ts)) as u64);
                }
            }
        }
        (b"trak", Ctx::Moov) => {
            p.track = TrackInfo::default();
            p.track_timescale = 0;
            walk(r, p, payload, end, depth + 1, Ctx::Trak)?;
            if p.track.kind != TrackKind::Other || p.track.width > 0 {
                p.info.tracks.push(std::mem::take(&mut p.track));
            }
        }
        (b"tkhd", Ctx::Trak) => {
            let v = leaf(r, p, payload, end, 128)?;
            if v.len() >= 8 {
                let version = v[0];
                let base = if version == 1 { 4 + 8 + 8 } else { 4 + 4 + 4 };
                if v.len() > base + 4 {
                    p.track.track_id = u32be(&v, base);
                }
                // duration, in the movie timescale: it sits right after
                // track_ID(4) + reserved(4) for both box versions
                let dur_off = base + 8;
                if version == 1 && v.len() >= dur_off + 8 {
                    let d = u64::from_be_bytes([
                        v[dur_off],
                        v[dur_off + 1],
                        v[dur_off + 2],
                        v[dur_off + 3],
                        v[dur_off + 4],
                        v[dur_off + 5],
                        v[dur_off + 6],
                        v[dur_off + 7],
                    ]);
                    if p.mvhd_timescale > 0 {
                        p.track.duration_ms = Some(d * 1000 / u64::from(p.mvhd_timescale));
                    }
                } else if version == 0 && v.len() >= dur_off + 4 {
                    let d = u32be(&v, dur_off);
                    if p.mvhd_timescale > 0 {
                        p.track.duration_ms =
                            Some(u64::from(d) * 1000 / u64::from(p.mvhd_timescale));
                    }
                }
                // matrix + width/height
                let m_off = if version == 1 {
                    4 + 8 + 8 + 4 + 4 + 8 + 8 + 2 + 2 + 2 + 2
                } else {
                    4 + 4 + 4 + 4 + 4 + 4 + 8 + 2 + 2 + 2 + 2
                };
                if v.len() >= m_off + 36 + 8 {
                    let a = fixed_16_16(i32be(&v, m_off));
                    let b = fixed_16_16(i32be(&v, m_off + 4));
                    p.track.rotation = rotation_from_matrix(a, b);
                    let w = fixed_16_16(i32be(&v, m_off + 36));
                    let h = fixed_16_16(i32be(&v, m_off + 40));
                    if p.track.width == 0 {
                        p.track.width = w.round() as u32;
                    }
                    if p.track.height == 0 {
                        p.track.height = h.round() as u32;
                    }
                }
            }
        }
        (b"mdia", Ctx::Trak) => walk(r, p, payload, end, depth + 1, Ctx::Mdia)?,
        (b"mdhd", Ctx::Mdia) => {
            let v = leaf(r, p, payload, end, 64)?;
            if v.len() >= 24 {
                let version = v[0];
                if version == 1 {
                    if v.len() >= 4 + 8 + 8 + 4 + 8 + 2 {
                        p.track_timescale = u32be(&v, 20);
                        let d = u64::from_be_bytes([
                            v[24], v[25], v[26], v[27], v[28], v[29], v[30], v[31],
                        ]);
                        if p.track_timescale > 0 {
                            p.track.duration_ms = Some(d * 1000 / u64::from(p.track_timescale));
                        }
                        p.track.language = some_if_nonempty(unpack_language(u16be(&v, 32)));
                    }
                } else if v.len() >= 4 + 4 + 4 + 4 + 4 + 2 {
                    p.track_timescale = u32be(&v, 12);
                    let d = u32be(&v, 16);
                    if p.track_timescale > 0 {
                        p.track.duration_ms =
                            Some(u64::from(d) * 1000 / u64::from(p.track_timescale));
                    }
                    p.track.language = some_if_nonempty(unpack_language(u16be(&v, 20)));
                }
            }
        }
        (b"hdlr", Ctx::Mdia) => {
            let v = leaf(r, p, payload, end, 64)?;
            if v.len() >= 12 {
                let h = [v[8], v[9], v[10], v[11]];
                p.track.kind = match &h {
                    b"vide" => TrackKind::Video,
                    b"soun" => TrackKind::Audio,
                    b"sbtl" | b"subt" | b"text" | b"tx3g" => TrackKind::Subtitle,
                    _ => TrackKind::Other,
                };
            }
        }
        (b"minf", Ctx::Mdia) => walk(r, p, payload, end, depth + 1, Ctx::Minf)?,
        (b"stbl", Ctx::Minf) => walk(r, p, payload, end, depth + 1, Ctx::Stbl)?,
        (b"stsd", Ctx::Stbl) => parse_stsd(r, p, payload, end, depth)?,
        // ---- spherical metadata ----
        (b"st3d", Ctx::SampleEntry) => {
            let v = leaf(r, p, payload, end, 16)?;
            if v.len() >= 5 {
                let (stereo, swap) = match v[4] {
                    0 => (Some(StereoLayout::Mono), false),
                    1 => (Some(StereoLayout::TopBottom), false),
                    2 => (Some(StereoLayout::SideBySide), false),
                    3 => (None, false), // stereo-custom (mesh)
                    4 => (Some(StereoLayout::SideBySide), true), // right-left
                    _ => (None, false),
                };
                let s = ensure_spherical(&mut p.track);
                s.stereo = s.stereo.or(stereo);
                s.swap_eyes = s.swap_eyes || swap;
                s.version = Some(SphericalVersion::V2Boxes);
            }
        }
        (b"sv3d", _) => {
            // `sv3d` on its own already tells us the track is spherical; the
            // `proj` children refine the projection kind.
            {
                let s = ensure_spherical(&mut p.track);
                if matches!(s.projection, ProjectionKind::Unknown) {
                    s.projection = ProjectionKind::Equirectangular;
                }
                s.version = Some(SphericalVersion::V2Boxes);
            }
            walk(r, p, payload, end, depth + 1, Ctx::Sv3d)?;
        }
        (b"svhd", Ctx::Sv3d) => {
            let s = ensure_spherical(&mut p.track);
            s.version = Some(SphericalVersion::V2Boxes);
        }
        (b"proj", Ctx::Sv3d) => walk(r, p, payload, end, depth + 1, Ctx::Proj)?,
        (b"prhd", Ctx::Proj) => {
            let v = leaf(r, p, payload, end, 32)?;
            if v.len() >= 16 {
                let yaw = fixed_16_16(i32be(&v, 4));
                let pitch = fixed_16_16(i32be(&v, 8));
                let roll = fixed_16_16(i32be(&v, 12));
                let s = ensure_spherical(&mut p.track);
                s.pose = Some(Pose { yaw, pitch, roll });
                s.version = Some(SphericalVersion::V2Boxes);
            }
        }
        (b"equi", Ctx::Proj) => {
            let v = leaf(r, p, payload, end, 32)?;
            if v.len() >= 20 {
                let s = ensure_spherical(&mut p.track);
                s.projection = ProjectionKind::Equirectangular;
                s.crop = Some(Crop {
                    top: fixed_0_32(u32be(&v, 4)),
                    bottom: fixed_0_32(u32be(&v, 8)),
                    cropped_left: fixed_0_32(u32be(&v, 12)),
                    right: fixed_0_32(u32be(&v, 16)),
                });
                s.version = Some(SphericalVersion::V2Boxes);
            }
        }
        (b"cbmp", Ctx::Proj) => {
            let s = ensure_spherical(&mut p.track);
            s.projection = ProjectionKind::Cubemap;
            s.version = Some(SphericalVersion::V2Boxes);
        }
        (b"mshp", Ctx::Proj) => {
            let s = ensure_spherical(&mut p.track);
            s.projection = ProjectionKind::Mesh;
            s.version = Some(SphericalVersion::V2Boxes);
        }
        (b"uuid", _) => parse_uuid(r, p, payload, end)?,
        _ => {
            // container boxes we still want to descend into
            if matches!(
                &fourcc,
                b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"udta" | b"meta"
            ) && ctx != Ctx::Top
            {
                walk(r, p, payload, end, depth + 1, ctx)?;
            }
        }
    }
    Ok(())
}

fn some_if_nonempty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
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

/// Display rotation (Android convention: clockwise degrees to apply) from the
/// `a`/`b` terms of the track header matrix.
pub fn rotation_from_matrix(a: f32, b: f32) -> i32 {
    if a == 0.0 && b == 0.0 {
        return 0;
    }
    let deg = b.atan2(a).to_degrees();
    let snapped = ((deg / 90.0).round() * 90.0) as i32;
    snapped.rem_euclid(360)
}

fn parse_stsd<R: Read + Seek>(
    r: &mut R,
    p: &mut Parse,
    payload: u64,
    end: u64,
    depth: u32,
) -> Result<(), ProbeError> {
    let mut hdr = [0u8; 8];
    if payload + 8 > end || read_at(r, payload, &mut hdr, p).is_err() {
        return Ok(());
    }
    let count = u32be(&hdr, 4).min(16);
    let mut pos = payload + 8;
    for _ in 0..count {
        if pos + 8 > end || depth > MAX_DEPTH {
            break;
        }
        let mut ehdr = [0u8; 8];
        if read_at(r, pos, &mut ehdr, p).is_err() {
            break;
        }
        let size = u32::from_be_bytes([ehdr[0], ehdr[1], ehdr[2], ehdr[3]]) as u64;
        let fourcc = [ehdr[4], ehdr[5], ehdr[6], ehdr[7]];
        if size < 8 || pos + size > end {
            break;
        }
        let entry_end = pos + size;
        let body = pos + 8;
        if is_video_entry(&fourcc) {
            p.in_video_entry = true;
            let v = leaf(r, p, body, entry_end, 96)?;
            if v.len() >= 78 {
                p.track.codec = Some(fourcc_str(fourcc));
                p.track.width = u16be(&v, 24) as u32;
                p.track.height = u16be(&v, 26) as u32;
                if p.track.kind == TrackKind::Other {
                    p.track.kind = TrackKind::Video;
                }
            }
            if body + 78 < entry_end {
                walk(r, p, body + 78, entry_end, depth + 1, Ctx::SampleEntry)?;
            }
            p.in_video_entry = false;
        } else if is_audio_entry(&fourcc) {
            let v = leaf(r, p, body, entry_end, 64)?;
            if v.len() >= 28 {
                if p.track.kind == TrackKind::Video {
                    // a second track: keep the existing video one intact
                } else {
                    p.track.codec = Some(fourcc_str(fourcc));
                    p.track.channels = u16be(&v, 16) as u32;
                    p.track.sample_rate = u32be(&v, 24) >> 16;
                    if p.track.kind == TrackKind::Other {
                        p.track.kind = TrackKind::Audio;
                    }
                }
            }
        }
        pos = entry_end;
    }
    Ok(())
}

fn parse_uuid<R: Read + Seek>(
    r: &mut R,
    p: &mut Parse,
    payload: u64,
    end: u64,
) -> Result<(), ProbeError> {
    if payload + 16 > end {
        return Ok(());
    }
    let mut uuid = [0u8; 16];
    if read_at(r, payload, &mut uuid, p).is_err() {
        return Ok(());
    }
    if uuid != SPHERICAL_V1_UUID {
        return Ok(());
    }
    let v = leaf(r, p, payload + 16, end, 16 * 1024)?;
    let xml = String::from_utf8_lossy(&v);
    if let Some(meta) = parse_v1_xml(&xml) {
        let s = ensure_spherical(&mut p.track);
        if s.stereo.is_none() {
            s.stereo = meta.0;
        }
        if matches!(s.projection, ProjectionKind::Unknown) {
            s.projection = meta.1;
        }
        if s.pose.is_none() {
            s.pose = meta.2;
        }
        if s.version.is_none() {
            s.version = Some(SphericalVersion::V1Xml);
        }
    }
    Ok(())
}

/// Extract the interesting bits of a Spherical Video V1 XML payload.
///
/// Returns `(stereo, projection, pose)`.
pub fn parse_v1_xml(xml: &str) -> Option<(Option<StereoLayout>, ProjectionKind, Option<Pose>)> {
    if !xml.contains("SphericalVideo") && !xml.contains("GSpherical") {
        return None;
    }
    let stereo = match tag_value(xml, "StereoMode")?.as_str() {
        "left-right" => Some(StereoLayout::SideBySide),
        "top-bottom" => Some(StereoLayout::TopBottom),
        "mono" => Some(StereoLayout::Mono),
        _ => None,
    };
    let projection = match tag_value(xml, "ProjectionType")?.as_str() {
        "equirectangular" => ProjectionKind::Equirectangular,
        "cubemap" => ProjectionKind::Cubemap,
        "mesh" => ProjectionKind::Mesh,
        _ => ProjectionKind::Unknown,
    };
    let yaw = tag_value(xml, "InitialViewPointDegrees").and_then(|v| v.parse::<f32>().ok());
    let pitch = tag_value(xml, "InitialViewPointDegreesY")
        .or_else(|| tag_value(xml, "InitialViewPointElevationDegrees"))
        .and_then(|v| v.parse::<f32>().ok());
    let roll = tag_value(xml, "InitialViewPointDegreesZ")
        .or_else(|| tag_value(xml, "InitialViewRollDegrees"))
        .and_then(|v| v.parse::<f32>().ok());
    let pose = match (yaw, pitch, roll) {
        (Some(y), Some(p), Some(r)) => Some(Pose {
            yaw: y,
            pitch: p,
            roll: r,
        }),
        (Some(y), _, _) => Some(Pose {
            yaw: y,
            pitch: 0.0,
            roll: 0.0,
        }),
        _ => None,
    };
    Some((stereo, projection, pose))
}

/// Read `<ns:Tag>value</ns:Tag>` from a small XML blob.
pub fn tag_value(xml: &str, tag: &str) -> Option<String> {
    // tolerate any namespace prefix
    let mut search_from = 0;
    loop {
        let idx = xml[search_from..].find(tag)? + search_from;
        // must be preceded by '<' (possibly with a namespace) and followed by '>'
        let open_start = xml[..idx].rfind('<')?;
        let gt = xml[idx..].find('>')? + idx;
        let open = &xml[open_start..gt + 1];
        if !open.starts_with('<') || open.starts_with("</") {
            search_from = gt + 1;
            continue;
        }
        let close_tag = format!("</{}", &open[1..open.find(':').map(|i| i + 1).unwrap_or(1)]);
        let _ = close_tag;
        let close = xml[gt + 1..].find("</")? + gt + 1;
        let value = xml[gt + 1..close].trim().to_string();
        if value.is_empty() {
            return None;
        }
        return Some(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect_container;
    use vlcrs_vr::Coverage;

    /// Minimal MP4 writer used by the tests.
    struct Mp4(Vec<u8>);

    impl Mp4 {
        fn new() -> Self {
            Mp4(Vec::new())
        }
        fn box_begin(&mut self, fourcc: &[u8; 4]) -> usize {
            let at = self.0.len();
            self.0.extend_from_slice(&[0, 0, 0, 0]);
            self.0.extend_from_slice(fourcc);
            at
        }
        fn box_end(&mut self, at: usize) {
            let size = (self.0.len() - at) as u32;
            self.0[at..at + 4].copy_from_slice(&size.to_be_bytes());
        }
        fn leaf(&mut self, fourcc: &[u8; 4], body: &[u8]) {
            let at = self.box_begin(fourcc);
            self.0.extend_from_slice(body);
            self.box_end(at);
        }
        fn u8(&mut self, v: u8) {
            self.0.push(v);
        }
        fn u16(&mut self, v: u16) {
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        fn u32(&mut self, v: u32) {
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        fn i32(&mut self, v: i32) {
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        fn i16(&mut self, v: i16) {
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        fn pad(&mut self, n: usize) {
            let at = self.0.len();
            self.0.resize(at + n, 0);
        }
        fn ftyp(&mut self, brand: &str) {
            let at = self.box_begin(b"ftyp");
            self.0.extend_from_slice(brand.as_bytes());
            self.u32(0);
            self.0.extend_from_slice(b"isom");
            self.box_end(at);
        }
        fn mvhd(&mut self, timescale: u32, duration: u32) {
            let at = self.box_begin(b"mvhd");
            self.u32(0); // version + flags
            self.u32(0);
            self.u32(0);
            self.u32(timescale);
            self.u32(duration);
            self.box_end(at);
        }
        fn tkhd(&mut self, track_id: u32, matrix_ab: (f32, f32), wh: (u32, u32)) {
            let at = self.box_begin(b"tkhd");
            self.u32(0); // version + flags
            self.u32(0); // creation
            self.u32(0); // modification
            self.u32(track_id);
            self.u32(0); // reserved
            self.u32(0); // duration
            self.pad(8); // reserved
            self.u16(0); // layer
            self.u16(0); // alternate group
            self.u16(0); // volume
            self.u16(0); // reserved
                         // matrix 3x3 (16.16 / 2.30)
            let a = (matrix_ab.0 * 65536.0) as i32;
            let b = (matrix_ab.1 * 65536.0) as i32;
            self.i32(a);
            self.i32(b);
            self.i32(0);
            self.i32(-b);
            self.i32(a);
            self.i32(0);
            self.i32(0);
            self.i32(0);
            self.i32(0x40000000);
            self.u32(wh.0 * 65536);
            self.u32(wh.1 * 65536);
            self.box_end(at);
        }
        fn mdhd(&mut self, timescale: u32, duration: u32) {
            let at = self.box_begin(b"mdhd");
            self.u32(0);
            self.u32(0);
            self.u32(0);
            self.u32(timescale);
            self.u32(duration);
            self.u16(0x55c4); // "und"
            self.u16(0);
            self.box_end(at);
        }
        fn hdlr(&mut self, kind: &[u8; 4]) {
            let at = self.box_begin(b"hdlr");
            self.u32(0);
            self.u32(0);
            self.0.extend_from_slice(kind);
            self.pad(12);
            self.box_end(at);
        }
        fn stsd_video(&mut self, codec: &[u8; 4], w: u16, h: u16, children: &[u8]) {
            let stsd = self.box_begin(b"stsd");
            self.u32(0);
            self.u32(1); // entry count
            let entry = self.box_begin(codec);
            self.pad(6); // reserved
            self.u16(1); // data reference index
            self.pad(16); // pre_defined + reserved
            self.u16(w);
            self.u16(h);
            self.u32(0x00480000);
            self.u32(0x00480000);
            self.u32(0);
            self.u16(1);
            self.pad(32);
            self.u16(0x0018);
            self.i16(-1);
            self.0.extend_from_slice(children);
            self.box_end(entry);
            self.box_end(stsd);
        }
        fn stsd_audio(&mut self, codec: &[u8; 4], channels: u16, rate: u32) {
            let stsd = self.box_begin(b"stsd");
            self.u32(0);
            self.u32(1);
            let entry = self.box_begin(codec);
            self.pad(6);
            self.u16(1);
            self.pad(8);
            self.u16(channels);
            self.u16(16);
            self.u16(0);
            self.u16(0);
            self.u32(rate * 65536);
            self.box_end(entry);
            self.box_end(stsd);
        }
    }

    /// Children of a video sample entry carrying spherical V2 metadata.
    fn spherical_children(
        stereo_mode: u8,
        pose: Option<(f32, f32, f32)>,
        bounds: Option<[u32; 4]>,
    ) -> Vec<u8> {
        let mut c = Mp4::new();
        // st3d
        let at = c.box_begin(b"st3d");
        c.u32(0);
        c.u8(stereo_mode);
        c.box_end(at);
        // sv3d > svhd + proj > prhd + equi
        let sv3d = c.box_begin(b"sv3d");
        let svhd = c.box_begin(b"svhd");
        c.u32(0);
        c.0.extend_from_slice(b"libvlcrs test");
        c.u8(0);
        c.box_end(svhd);
        let proj = c.box_begin(b"proj");
        let prhd = c.box_begin(b"prhd");
        c.u32(0);
        let (y, pi, ro) = pose.unwrap_or((0.0, 0.0, 0.0));
        c.i32((y * 65536.0) as i32);
        c.i32((pi * 65536.0) as i32);
        c.i32((ro * 65536.0) as i32);
        c.box_end(prhd);
        if let Some(b) = bounds {
            let equi = c.box_begin(b"equi");
            c.u32(0);
            for v in b {
                c.u32(v);
            }
            c.box_end(equi);
        }
        c.box_end(proj);
        c.box_end(sv3d);
        c.0
    }

    fn build_mp4(children: &[u8], rotation: (f32, f32), wh: (u32, u32)) -> Vec<u8> {
        let mut f = Mp4::new();
        f.ftyp("isom");
        let moov = f.box_begin(b"moov");
        f.mvhd(1000, 12_345);
        let trak = f.box_begin(b"trak");
        f.tkhd(1, rotation, wh);
        let mdia = f.box_begin(b"mdia");
        f.mdhd(90_000, 90_000 * 10);
        f.hdlr(b"vide");
        let minf = f.box_begin(b"minf");
        let stbl = f.box_begin(b"stbl");
        f.stsd_video(b"avc1", wh.0 as u16, wh.1 as u16, children);
        f.box_end(stbl);
        f.box_end(minf);
        f.box_end(mdia);
        f.box_end(trak);
        // audio track
        let trak = f.box_begin(b"trak");
        f.tkhd(2, (1.0, 0.0), (0, 0));
        let mdia = f.box_begin(b"mdia");
        f.mdhd(44_100, 44_100 * 10);
        f.hdlr(b"soun");
        let minf = f.box_begin(b"minf");
        let stbl = f.box_begin(b"stbl");
        f.stsd_audio(b"mp4a", 2, 44_100);
        f.box_end(stbl);
        f.box_end(minf);
        f.box_end(mdia);
        f.box_end(trak);
        f.box_end(moov);
        f.0
    }

    fn probe(bytes: &[u8]) -> MediaInfo {
        match crate::probe_bytes(bytes) {
            Ok(i) => i,
            Err(e) => panic!("probe failed: {e:?} (len {})", bytes.len()),
        }
    }

    #[test]
    fn detects_container_and_basic_info() {
        let bytes = build_mp4(&[], (1.0, 0.0), (1920, 1080));
        assert_eq!(detect_container(&bytes), Container::Mp4);
        let info = probe(&bytes);
        assert_eq!(info.container, Container::Mp4);
        assert_eq!(info.brand.as_deref(), Some("isom"));
        assert_eq!(info.duration_ms, Some(12_345));
        assert_eq!(info.tracks.len(), 2);
        let v = info.video().expect("video track");
        assert_eq!(v.codec.as_deref(), Some("avc1"));
        assert_eq!((v.width, v.height), (1920, 1080));
        assert_eq!(v.rotation, 0);
        assert!(v.spherical.is_none());
        let a = info.audio().expect("audio track");
        assert_eq!(a.codec.as_deref(), Some("mp4a"));
        assert_eq!(a.channels, 2);
        assert_eq!(a.sample_rate, 44_100);
    }

    #[test]
    fn rotation_from_tkhd_matrix() {
        // Android convention: rotate 90° clockwise for display
        let bytes = build_mp4(&[], (0.0, 1.0), (1080, 1920));
        let info = probe(&bytes);
        assert_eq!(info.video().unwrap().rotation, 90);
        let bytes = build_mp4(&[], (-1.0, 0.0), (1920, 1080));
        assert_eq!(probe(&bytes).video().unwrap().rotation, 180);
        let bytes = build_mp4(&[], (0.0, -1.0), (1920, 1080));
        assert_eq!(probe(&bytes).video().unwrap().rotation, 270);
        assert_eq!(rotation_from_matrix(1.0, 0.0), 0);
    }

    #[test]
    fn reads_st3d_and_sv3d_mono_360() {
        let children = spherical_children(0, None, None);
        let info = probe(&build_mp4(&children, (1.0, 0.0), (3840, 1920)));
        let s = info.video().unwrap().spherical.unwrap();
        assert_eq!(s.stereo, Some(StereoLayout::Mono));
        assert!(!s.swap_eyes);
        assert_eq!(s.projection, ProjectionKind::Equirectangular);
        assert_eq!(s.version, Some(SphericalVersion::V2Boxes));
        // 2:1 equirect without bounds → 360°
        assert_eq!(s.coverage(3840, 1920), Some(Coverage::Full360));
        assert!(s.crop.is_none(), "no equi box → no bounds metadata");
    }

    #[test]
    fn reads_st3d_left_right() {
        let children = spherical_children(2, Some((30.0, -10.0, 5.0)), None);
        let info = probe(&build_mp4(&children, (1.0, 0.0), (3840, 1080)));
        let s = info.video().unwrap().spherical.unwrap();
        assert_eq!(s.stereo, Some(StereoLayout::SideBySide));
        assert!(!s.swap_eyes);
        let pose = s.pose.unwrap();
        assert!((pose.yaw - 30.0).abs() < 1e-3);
        assert!((pose.pitch + 10.0).abs() < 1e-3);
        assert!((pose.roll - 5.0).abs() < 1e-3);
        // each eye is 1920x1080 → 16:9, not 1:1 → treated as 360 SBS
        assert_eq!(s.coverage(3840, 1080), Some(Coverage::Full360));
    }

    #[test]
    fn reads_st3d_right_left_as_swapped() {
        let children = spherical_children(4, None, None);
        let s = probe(&build_mp4(&children, (1.0, 0.0), (3840, 1920)))
            .video()
            .unwrap()
            .spherical
            .unwrap();
        assert_eq!(s.stereo, Some(StereoLayout::SideBySide));
        assert!(s.swap_eyes);
    }

    #[test]
    fn reads_st3d_top_bottom() {
        let children = spherical_children(1, None, None);
        let s = probe(&build_mp4(&children, (1.0, 0.0), (1920, 1920)))
            .video()
            .unwrap()
            .spherical
            .unwrap();
        assert_eq!(s.stereo, Some(StereoLayout::TopBottom));
        assert!(!s.swap_eyes);
        // per eye 1920x960 = 2:1 → 360
        assert_eq!(s.coverage(1920, 1920), Some(Coverage::Full360));
    }

    #[test]
    fn reads_equi_bounds_as_180() {
        // left and right crops of 25% each → 180° horizontal coverage
        let q = (0.25f64 * (1u64 << 32) as f64) as u32;
        let children = spherical_children(0, None, Some([0, 0, q, q]));
        let s = probe(&build_mp4(&children, (1.0, 0.0), (1920, 1920)))
            .video()
            .unwrap()
            .spherical
            .unwrap();
        assert_eq!(s.projection, ProjectionKind::Equirectangular);
        let crop = s.crop.expect("equi bounds");
        assert!((crop.cropped_left - 0.25).abs() < 1e-6);
        assert!((crop.right - 0.25).abs() < 1e-6);
        assert!((s.coverage_degrees().unwrap() - 180.0).abs() < 0.01);
        assert_eq!(s.coverage(1920, 1920), Some(Coverage::Half180));
        assert!(!s.coverage_from_aspect);
    }

    #[test]
    fn aspect_decides_coverage_when_bounds_are_absent() {
        // mono: 1:1 → 180°, 2:1 → 360°
        let children = spherical_children(0, None, None);
        let s = probe(&build_mp4(&children, (1.0, 0.0), (1920, 1920)))
            .video()
            .unwrap()
            .spherical
            .unwrap();
        assert!(s.crop.is_none());
        assert_eq!(s.coverage(1920, 1920), Some(Coverage::Half180));
        assert_eq!(s.coverage(3840, 1920), Some(Coverage::Full360));
        // SBS with a 2:1 frame is the "squeezed eyes" 360 convention: it is
        // indistinguishable from an unsqueezed 180 SBS without crop bounds, so
        // 360 wins (and the user can always force 180 SBS explicitly).
        let children = spherical_children(2, None, None);
        let s = probe(&build_mp4(&children, (1.0, 0.0), (3840, 1920)))
            .video()
            .unwrap()
            .spherical
            .unwrap();
        assert_eq!(s.stereo, Some(StereoLayout::SideBySide));
        assert_eq!(s.coverage(3840, 1920), Some(Coverage::Full360));
        // a 1:1 SBS frame is a 180 dome (each eye 1:2)
        assert_eq!(s.coverage(1920, 1920), Some(Coverage::Half180));
        // bounds always win over the geometry rule
        let q = (0.25f64 * (1u64 << 32) as f64) as u32;
        let children = spherical_children(2, None, Some([0, 0, q, q]));
        let s = probe(&build_mp4(&children, (1.0, 0.0), (3840, 1920)))
            .video()
            .unwrap()
            .spherical
            .unwrap();
        assert_eq!(s.coverage(3840, 1920), Some(Coverage::Half180));
    }

    #[test]
    fn zero_bounds_are_authoritative_360() {
        // "For an uncropped frame all values are 0" — an explicit all-zero
        // `equi` box means the frame covers the whole sphere, even when the
        // picture geometry would suggest 180°.
        let children = spherical_children(0, None, Some([0, 0, 0, 0]));
        let s = probe(&build_mp4(&children, (1.0, 0.0), (1920, 1920)))
            .video()
            .unwrap()
            .spherical
            .unwrap();
        let crop = s.crop.expect("equi present");
        assert!(crop.is_empty());
        assert!((s.coverage_degrees().unwrap() - 360.0).abs() < 1e-3);
        assert_eq!(s.coverage(1920, 1920), Some(Coverage::Full360));
    }

    #[test]
    fn stereo_aspect_rules() {
        // 2:1 frame, SBS → 360 (the common "squeezed eyes" convention)
        let children = spherical_children(2, None, None);
        let s = probe(&build_mp4(&children, (1.0, 0.0), (3840, 1920)))
            .video()
            .unwrap()
            .spherical
            .unwrap();
        assert_eq!(s.coverage(3840, 1920), Some(Coverage::Full360));
        // 2:1 frame, TB → 360 (two 2:1 eyes stacked into 1:1 would be 180)
        let children = spherical_children(1, None, None);
        let s = probe(&build_mp4(&children, (1.0, 0.0), (1920, 1920)))
            .video()
            .unwrap()
            .spherical
            .unwrap();
        assert_eq!(s.coverage(1920, 1920), Some(Coverage::Full360));
        // 1:2 frame, TB → 180 (two 1:1 eyes stacked)
        let s2 = SphericalMetadata {
            projection: ProjectionKind::Equirectangular,
            stereo: Some(StereoLayout::TopBottom),
            ..Default::default()
        };
        assert_eq!(s2.coverage(1920, 3840), Some(Coverage::Half180));
        // 1:1 frame, mono → 180
        let s3 = SphericalMetadata {
            projection: ProjectionKind::Equirectangular,
            stereo: Some(StereoLayout::Mono),
            ..Default::default()
        };
        assert_eq!(s3.coverage(1920, 1920), Some(Coverage::Half180));
        assert_eq!(s3.coverage(3840, 1920), Some(Coverage::Full360));
    }

    #[test]
    fn reads_cubemap_and_mesh_projections() {
        for (fourcc, expected) in [
            (b"cbmp", ProjectionKind::Cubemap),
            (b"mshp", ProjectionKind::Mesh),
        ] {
            let mut c = Mp4::new();
            let sv3d = c.box_begin(b"sv3d");
            let proj = c.box_begin(b"proj");
            let leaf_at = c.box_begin(fourcc);
            c.u32(0);
            c.u32(0);
            c.u32(0);
            c.box_end(leaf_at);
            c.box_end(proj);
            c.box_end(sv3d);
            let s = probe(&build_mp4(&c.0, (1.0, 0.0), (1920, 1080)))
                .video()
                .unwrap()
                .spherical
                .unwrap();
            assert_eq!(s.projection, expected);
            assert!(!s.projection.supported());
            assert_eq!(s.coverage(1920, 1080), None);
        }
    }

    #[test]
    fn reads_spherical_v1_uuid_xml() {
        let xml = r#"<?xml version="1.0"?>
<rdf:SphericalVideo xmlns:GSpherical="http://ns.google.com/videos/1.0/spherical/">
 <GSpherical:SphericalVideo>true</GSpherical:SphericalVideo>
 <GSpherical:Spherical>true</GSpherical:Spherical>
 <GSpherical:Stitched>true</GSpherical:Stitched>
 <GSpherical:ProjectionType>equirectangular</GSpherical:ProjectionType>
 <GSpherical:StereoMode>top-bottom</GSpherical:StereoMode>
 <GSpherical:InitialViewPointDegrees>15</GSpherical:InitialViewPointDegrees>
</rdf:SphericalVideo>"#;
        let mut payload = Vec::new();
        payload.extend_from_slice(&SPHERICAL_V1_UUID);
        payload.extend_from_slice(xml.as_bytes());
        let mut c = Mp4::new();
        c.leaf(b"uuid", &payload);
        let info = probe(&build_mp4(&c.0, (1.0, 0.0), (3840, 1920)));
        let s = info.video().unwrap().spherical.unwrap();
        assert_eq!(s.version, Some(SphericalVersion::V1Xml));
        assert_eq!(s.stereo, Some(StereoLayout::TopBottom));
        assert_eq!(s.projection, ProjectionKind::Equirectangular);
        assert!((s.pose.unwrap().yaw - 15.0).abs() < 1e-3);
        assert_eq!(s.coverage(3840, 1920), Some(Coverage::Full360));
    }

    #[test]
    fn v1_xml_tag_extraction() {
        let xml = "<a:Tag> value </a:Tag><b:Other>x</b:Other>";
        assert_eq!(tag_value(xml, "Tag").as_deref(), Some("value"));
        assert_eq!(tag_value(xml, "Other").as_deref(), Some("x"));
        assert_eq!(tag_value(xml, "Missing"), None);
        assert!(parse_v1_xml("<html/>").is_none());
    }

    #[test]
    fn hints_are_exposed_for_the_renderer() {
        // 180 SBS: bounds crop 25% off each side of the sphere
        let q = (0.25f64 * (1u64 << 32) as f64) as u32;
        let children = spherical_children(2, Some((0.0, 0.0, 0.0)), Some([0, 0, q, q]));
        let info = probe(&build_mp4(&children, (1.0, 0.0), (3840, 1920)));
        let h = info.hints();
        assert_eq!(h.width, 3840);
        assert_eq!(h.height, 1920);
        assert_eq!(h.stereo, Some(StereoLayout::SideBySide));
        assert_eq!(h.coverage, Some(Coverage::Half180));
        assert!(h.has_metadata());
        assert_eq!(h.pose, Some(Pose::default()));
        // platform overrides win
        let h2 = info.hints_with(4096, 2048, 90);
        assert_eq!((h2.width, h2.height, h2.rotation), (4096, 2048, 90));
        assert_eq!(h2.coverage, Some(Coverage::Half180));
    }

    #[test]
    fn truncated_and_garbage_files_do_not_panic() {
        let bytes = build_mp4(&[], (1.0, 0.0), (1920, 1080));
        for cut in [
            0usize,
            1,
            7,
            8,
            20,
            64,
            200,
            bytes.len() / 2,
            bytes.len() - 1,
        ] {
            let mut c = std::io::Cursor::new(&bytes[..cut.min(bytes.len())]);
            let _ = crate::probe(&mut c);
        }
        // random garbage
        let junk = vec![0xABu8; 4096];
        let mut c = std::io::Cursor::new(&junk);
        assert!(crate::probe(&mut c).is_err());
        // a valid header but a bogus box size
        let mut bad = Vec::new();
        bad.extend_from_slice(&8u32.to_be_bytes());
        bad.extend_from_slice(b"ftyp");
        bad.extend_from_slice(&u32::MAX.to_be_bytes());
        bad.extend_from_slice(b"moov");
        let mut c = std::io::Cursor::new(&bad);
        let _ = crate::probe(&mut c);
    }

    #[test]
    fn moov_at_the_end_is_found() {
        let mut f = Mp4::new();
        f.ftyp("mp42");
        // a big mdat first
        let at = f.box_begin(b"mdat");
        f.pad(4096);
        f.box_end(at);
        let children = spherical_children(0, None, None);
        let moov = f.box_begin(b"moov");
        f.mvhd(1000, 5000);
        let trak = f.box_begin(b"trak");
        f.tkhd(1, (1.0, 0.0), (3840, 1920));
        let mdia = f.box_begin(b"mdia");
        f.mdhd(90_000, 90_000);
        f.hdlr(b"vide");
        let minf = f.box_begin(b"minf");
        let stbl = f.box_begin(b"stbl");
        f.stsd_video(b"hvc1", 3840, 1920, &children);
        f.box_end(stbl);
        f.box_end(minf);
        f.box_end(mdia);
        f.box_end(trak);
        f.box_end(moov);
        let info = probe(&f.0);
        assert_eq!(info.brand.as_deref(), Some("mp42"));
        assert_eq!(info.video().unwrap().codec.as_deref(), Some("hvc1"));
        assert!(info.has_spherical());
        assert!(info.bytes_scanned > 0);
    }

    #[test]
    fn language_is_decoded() {
        let mut f = Mp4::new();
        f.ftyp("isom");
        let moov = f.box_begin(b"moov");
        let trak = f.box_begin(b"trak");
        f.tkhd(1, (1.0, 0.0), (0, 0));
        let mdia = f.box_begin(b"mdia");
        let at = f.box_begin(b"mdhd");
        f.u32(0);
        f.u32(0);
        f.u32(0);
        f.u32(48_000);
        f.u32(48_000);
        // "eng" packed: ((e-0x60)<<10) | ((n-0x60)<<5) | (g-0x60)
        let packed =
            (((b'e' - 0x60) as u16) << 10) | (((b'n' - 0x60) as u16) << 5) | ((b'g' - 0x60) as u16);
        f.u16(packed);
        f.u16(0);
        f.box_end(at);
        f.hdlr(b"soun");
        f.box_end(mdia);
        f.box_end(trak);
        f.box_end(moov);
        let info = probe(&f.0);
        assert_eq!(info.tracks[0].language.as_deref(), Some("eng"));
    }

    #[test]
    fn summary_is_readable() {
        let children = spherical_children(2, None, None);
        let info = probe(&build_mp4(&children, (1.0, 0.0), (3840, 1920)));
        let s = info.summary();
        assert!(s.contains("mp4(isom)"), "{s}");
        assert!(s.contains("3840x1920"), "{s}");
        assert!(s.contains("SideBySide"), "{s}");
        assert!(s.contains("audio"), "{s}");
    }
}
