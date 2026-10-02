//! Geometry generation: equirectangular sphere (360°), hemisphere (180°) and
//! the flat quad used by the forced-planar mode.
//!
//! Port of `xl_mesh_factory.c` with the longitude origin moved to the centre of
//! the picture (`lon = 0` → `u = 0.5`), which is what makes 180° sources line
//! up with the viewer at rest and keeps the yaw limits symmetric.

use crate::projection::{Coverage, UvRect};

/// Indexed triangle mesh with separate position / texcoord arrays.
///
/// Indices are 16 bit so the meshes can be uploaded on a GLES2 context without
/// `GL_OES_element_index_uint`.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    /// Vertex positions, 3 floats per vertex.
    pub positions: Vec<f32>,
    /// Texture coordinates in display space (`v = 0` bottom), 2 floats/vertex.
    pub texcoords: Vec<f32>,
    /// Triangle indices.
    pub indices: Vec<u16>,
}

impl Mesh {
    /// Number of vertices.
    #[inline]
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of indices (== 3 × triangle count).
    #[inline]
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }

    /// Interleaved `[x, y, z, u, v]` vertex array, handy for a single VBO.
    pub fn interleaved(&self) -> Vec<f32> {
        let n = self.vertex_count();
        let mut out = Vec::with_capacity(n * 5);
        for i in 0..n {
            out.push(self.positions[i * 3]);
            out.push(self.positions[i * 3 + 1]);
            out.push(self.positions[i * 3 + 2]);
            out.push(self.texcoords[i * 2]);
            out.push(self.texcoords[i * 2 + 1]);
        }
        out
    }
}

/// Default tessellation step in degrees.
///
/// 4° gives 24 300 indices for a full sphere: visually smooth on a phone panel
/// while staying well inside a 16 bit index buffer.
pub const DEFAULT_STEP_DEG: i32 = 4;

/// Coarsest step still considered acceptable (keeps indices in `u16`).
pub const MIN_STEP_DEG: i32 = 1;

/// Longitudinal overscan added on both sides of a 180° hemisphere.
///
/// A 180° equirectangular lune cannot cover a wide field of view near the
/// poles: as soon as the viewer tilts down (or up), the horizontal extent of
/// the frustum in *longitude* grows far beyond the ±90° of the picture.  Rather
/// than letting those rays miss the geometry (which would show black, and the
/// requirement explicitly forbids that) the hemisphere is tessellated over the
/// whole sphere and `u` is clamped to `[0, 1]`; the uncovered directions then
/// repeat the boundary column of the picture ("edge clamp").  Manual yaw and
/// pitch converge before that region is reached, so in practice it is only
/// visible when looking steeply up/down.
pub const HEMI_OVERSCAN_DEG: f32 = 90.0;

/// Clamp a tessellation step into the supported range.
pub fn sanitize_step(step: i32) -> i32 {
    step.clamp(MIN_STEP_DEG, 8)
}

/// Build the mesh for a coverage.
pub fn mesh_for(coverage: Coverage, step_deg: i32) -> Mesh {
    match coverage {
        Coverage::Planar => rect_mesh(),
        Coverage::Full360 => sphere_mesh(step_deg),
        Coverage::Half180 => hemisphere_mesh(step_deg, HEMI_OVERSCAN_DEG),
    }
}

/// Full equirectangular sphere (`lon ∈ [-180, 180]`, `u = 0.5 + lon/360`).
pub fn sphere_mesh(step_deg: i32) -> Mesh {
    equirect_mesh(-180.0, 180.0, 360.0, step_deg)
}

/// 180° hemisphere with longitudinal overscan (see [`HEMI_OVERSCAN_DEG`]).
///
/// `u = clamp(0.5 + lon/180, 0, 1)` so that the overscan repeats the boundary
/// column instead of showing black.
pub fn hemisphere_mesh(step_deg: i32, overscan_deg: f32) -> Mesh {
    let ov = overscan_deg.clamp(0.0, 90.0);
    equirect_mesh(-90.0 - ov, 90.0 + ov, 180.0, step_deg)
}

/// Flat quad covering clip space, texcoords in display space.
pub fn rect_mesh() -> Mesh {
    Mesh {
        positions: vec![
            -1.0, -1.0, 0.0, //
            1.0, -1.0, 0.0, //
            1.0, 1.0, 0.0, //
            -1.0, 1.0, 0.0, //
        ],
        texcoords: vec![
            0.0, 0.0, //
            1.0, 0.0, //
            1.0, 1.0, //
            0.0, 1.0, //
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
    }
}

/// Equirectangular patch spanning `lon ∈ [lon_start, lon_end]` and
/// `lat ∈ [-90, 90]`.
///
/// `lon_span` is the horizontal coverage of the *source picture* in degrees
/// (360 for a full sphere, 180 for a hemisphere) and drives the `u` mapping;
/// longitudes outside the coverage are clamped to the boundary column.
pub fn equirect_mesh(lon_start: f32, lon_end: f32, lon_span: f32, step_deg: i32) -> Mesh {
    let step = sanitize_step(step_deg);
    let lat_steps = (180 / step) as usize;
    let lon_steps = (((lon_end - lon_start) / step as f32).round()) as usize;
    let lon_steps = lon_steps.max(1);
    let cols = lon_steps + 1;
    let rows = lat_steps + 1;
    assert!(
        rows * cols <= u16::MAX as usize,
        "mesh too fine for u16 indices: {} vertices",
        rows * cols
    );

    let mut positions = Vec::with_capacity(rows * cols * 3);
    let mut texcoords = Vec::with_capacity(rows * cols * 2);

    for r in 0..rows {
        let lat = -90.0 + (r as f32) * step as f32;
        let lat_rad = lat.to_radians();
        let (sin_lat, cos_lat) = (lat_rad.sin(), lat_rad.cos());
        for c in 0..cols {
            let lon = lon_start + (c as f32) * step as f32;
            let lon_rad = lon.to_radians();
            // lon = 0 faces the camera (-Z), +lon goes to the viewer's right.
            let x = cos_lat * lon_rad.sin();
            let y = sin_lat;
            let z = -cos_lat * lon_rad.cos();
            positions.push(x);
            positions.push(y);
            positions.push(z);
            let u = 0.5 + lon / lon_span;
            texcoords.push(u.clamp(0.0, 1.0));
            texcoords.push(0.5 + lat / 180.0);
        }
    }

    let mut indices = Vec::with_capacity(lat_steps * lon_steps * 6);
    for r in 0..lat_steps {
        for c in 0..lon_steps {
            let i0 = (r * cols + c) as u16;
            let i1 = i0 + 1;
            let i2 = ((r + 1) * cols + c + 1) as u16;
            let i3 = ((r + 1) * cols + c) as u16;
            indices.push(i0);
            indices.push(i1);
            indices.push(i2);
            indices.push(i0);
            indices.push(i2);
            indices.push(i3);
        }
    }

    Mesh {
        positions,
        texcoords,
        indices,
    }
}

/// Longitude / latitude (degrees) of a mesh vertex.
pub fn vertex_lonlat(mesh: &Mesh, i: usize) -> (f32, f32) {
    let x = mesh.positions[i * 3];
    let y = mesh.positions[i * 3 + 1];
    let z = mesh.positions[i * 3 + 2];
    (
        crate::rad2deg(x.atan2(-z)),
        crate::rad2deg(crate::clampf(y, -1.0, 1.0).asin()),
    )
}

/// Apply an eye/layout sub-rectangle to a mesh's texcoords in place.
///
/// The renderer instead passes the rect as a uniform (so switching is free),
/// this helper exists for tests and for integrators that prefer baked coords.
pub fn apply_uv_rect(mesh: &mut Mesh, rect: UvRect) {
    for i in 0..mesh.texcoords.len() / 2 {
        let u = mesh.texcoords[i * 2];
        let v = mesh.texcoords[i * 2 + 1];
        mesh.texcoords[i * 2] = rect.u0 + u * rect.su;
        mesh.texcoords[i * 2 + 1] = rect.v0 + v * rect.sv;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::{Eye, StereoLayout};

    /// Longitude of a vertex, in degrees.
    fn lon(mesh: &Mesh, i: usize) -> f32 {
        let x = mesh.positions[i * 3];
        let z = mesh.positions[i * 3 + 2];
        crate::rad2deg(x.atan2(-z))
    }

    fn lat(mesh: &Mesh, i: usize) -> f32 {
        let y = mesh.positions[i * 3 + 1];
        crate::rad2deg(crate::clampf(y, -1.0, 1.0).asin())
    }

    #[test]
    fn sphere_counts_match_the_reference_formula() {
        // reference: step 5 → (180/5+1) * (360/5+1) vertices
        let m = sphere_mesh(5);
        assert_eq!(m.vertex_count(), (180 / 5 + 1) * (360 / 5 + 1));
        assert_eq!(m.index_count(), (360 / 5) * (180 / 5) * 6);
    }

    #[test]
    fn sphere_is_unit_radius() {
        let m = mesh_for(Coverage::Full360, DEFAULT_STEP_DEG);
        for i in 0..m.vertex_count() {
            let x = m.positions[i * 3];
            let y = m.positions[i * 3 + 1];
            let z = m.positions[i * 3 + 2];
            let r = (x * x + y * y + z * z).sqrt();
            assert!((r - 1.0).abs() < 1e-4, "radius {r} at vertex {i}");
        }
    }

    #[test]
    fn centre_of_the_picture_faces_the_camera() {
        // With the default 4° step the exact (0.5, 0.5) texel is not on the
        // grid, so look for the vertex closest to it.
        for cov in [Coverage::Full360, Coverage::Half180] {
            let m = mesh_for(cov, DEFAULT_STEP_DEG);
            let mut best = (f32::MAX, 0usize);
            for i in 0..m.vertex_count() {
                let du = (m.texcoords[i * 2] - 0.5).abs();
                let dv = (m.texcoords[i * 2 + 1] - 0.5).abs();
                if du + dv < best.0 {
                    best = (du + dv, i);
                }
            }
            let (_, i) = best;
            assert!(
                best.0 < 0.05,
                "no vertex near the picture centre for {cov:?}"
            );
            assert!(
                m.positions[i * 3].abs() < 0.15,
                "x = {}",
                m.positions[i * 3]
            );
            assert!(m.positions[i * 3 + 1].abs() < 0.15);
            assert!(
                m.positions[i * 3 + 2] < -0.9,
                "z = {}",
                m.positions[i * 3 + 2]
            );
        }
    }

    #[test]
    fn top_of_the_picture_is_up() {
        let m = mesh_for(Coverage::Full360, DEFAULT_STEP_DEG);
        for i in 0..m.vertex_count() {
            if (m.texcoords[i * 2 + 1] - 1.0).abs() < 1e-5 {
                assert!((m.positions[i * 3 + 1] - 1.0).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn positive_longitude_goes_right() {
        let m = mesh_for(Coverage::Full360, DEFAULT_STEP_DEG);
        for i in 0..m.vertex_count() {
            let u = m.texcoords[i * 2];
            let x = m.positions[i * 3];
            // near the poles x degenerates to ~0 for every longitude
            if x.abs() > 1e-3 && (u - 0.5).abs() > 1e-3 {
                assert_eq!((u > 0.5), (x > 0.0), "u = {u} vs x = {x}");
            }
        }
    }

    #[test]
    fn sphere_covers_all_longitudes() {
        let m = mesh_for(Coverage::Full360, DEFAULT_STEP_DEG);
        let mut min_lon = f32::MAX;
        let mut max_lon = f32::MIN;
        for i in 0..m.vertex_count() {
            // skip the poles, where longitude is degenerate
            if lat(&m, i).abs() > 89.0 {
                continue;
            }
            min_lon = min_lon.min(lon(&m, i));
            max_lon = max_lon.max(lon(&m, i));
        }
        assert!(min_lon <= -179.0, "min lon {min_lon}");
        assert!(max_lon >= 179.0, "max lon {max_lon}");
    }

    #[test]
    fn hemisphere_maps_the_lune_exactly() {
        let m = hemisphere_mesh(DEFAULT_STEP_DEG, 0.0);
        for i in 0..m.vertex_count() {
            let l = lon(&m, i);
            if l.abs() > 89.5 {
                continue;
            }
            let u = m.texcoords[i * 2];
            let expected = 0.5 + l / 180.0;
            assert!((u - expected).abs() < 1e-3, "lon {l} → u {u}");
            assert!(m.positions[i * 3 + 2] <= 1e-4, "must stay in front");
        }
    }

    #[test]
    fn hemisphere_overscan_prevents_holes() {
        // With the default overscan the geometry covers every direction, so a
        // 180° source can never show black; the region outside the lune simply
        // repeats the boundary column (u clamped to 0 / 1).
        let m = mesh_for(Coverage::Half180, DEFAULT_STEP_DEG);
        let mut min_lon = f32::MAX;
        let mut max_lon = f32::MIN;
        let mut clamped_left = 0;
        let mut clamped_right = 0;
        for i in 0..m.vertex_count() {
            if lat(&m, i).abs() > 89.0 {
                continue;
            }
            let l = lon(&m, i);
            min_lon = min_lon.min(l);
            max_lon = max_lon.max(l);
            let u = m.texcoords[i * 2];
            assert!((0.0..=1.0).contains(&u), "u out of range: {u}");
            if l.abs() >= 90.0 {
                // at |lon| >= 90 the vertex sits on (or beyond) a boundary and
                // must sample a boundary column.  The ±180 seam is ambiguous
                // when recovered from the position alone, so accept either end.
                assert!(
                    u.abs() < 1e-5 || (u - 1.0).abs() < 1e-5,
                    "lon {l} must clamp to a boundary column, got {u}"
                );
                if (-179.9..=-90.0).contains(&l) {
                    clamped_left += 1;
                    assert!(u.abs() < 1e-5, "lon {l} must clamp to u=0, got {u}");
                }
                if (90.0..179.9).contains(&l) {
                    clamped_right += 1;
                    assert!((u - 1.0).abs() < 1e-5, "lon {l} must clamp to u=1, got {u}");
                }
            }
        }
        assert!(min_lon <= -179.0, "min lon {min_lon}");
        assert!(max_lon >= 179.0, "max lon {max_lon}");
        assert!(clamped_left > 0 && clamped_right > 0);
    }

    #[test]
    fn hemisphere_u_stays_in_range_without_overscan() {
        let m = hemisphere_mesh(DEFAULT_STEP_DEG, 0.0);
        for i in 0..m.vertex_count() {
            assert!((0.0..=1.0).contains(&m.texcoords[i * 2]));
            assert!(
                m.positions[i * 3 + 2] <= 1e-4,
                "no geometry behind the viewer"
            );
        }
    }

    #[test]
    fn indices_stay_in_range_and_step_is_sanitized() {
        for step in [1, 2, 4, 5, 8] {
            let m = mesh_for(Coverage::Full360, step);
            let n = m.vertex_count();
            assert!(n <= u16::MAX as usize, "step {step} overflows u16");
            for idx in &m.indices {
                assert!((*idx as usize) < n);
            }
            // every triangle must be wound consistently enough to be drawn with
            // culling disabled: no degenerate index triples in the interior
            for t in m.indices.chunks(3) {
                assert_ne!(t[0], t[1]);
            }
        }
        assert_eq!(sanitize_step(0), MIN_STEP_DEG);
        assert_eq!(sanitize_step(999), 8);
    }

    #[test]
    fn interleaved_layout_is_five_floats() {
        let m = rect_mesh();
        let il = m.interleaved();
        assert_eq!(il.len(), m.vertex_count() * 5);
        assert_eq!(&il[0..5], &[-1.0, -1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn apply_uv_rect_matches_the_matrix() {
        let mut m = rect_mesh();
        let rect = UvRect::for_layout(StereoLayout::SideBySide, Eye::Right, false);
        apply_uv_rect(&mut m, rect);
        assert!((m.texcoords[0] - 0.5).abs() < 1e-6); // bottom-left → u = 0.5
        assert!((m.texcoords[2] - 1.0).abs() < 1e-6); // bottom-right → u = 1.0
    }
}
