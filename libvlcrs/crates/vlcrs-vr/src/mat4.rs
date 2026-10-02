//! 4×4 matrix helpers, column major, matching the GL layout expected by
//! `glUniformMatrix4fv(..., GL_FALSE, m)`.
//!
//! Port of `xl_mat4.c` (identical semantics: `mul(a, b) == a·b`,
//! `rotate_*` post-multiplies, i.e. `out = out·R`).

/// Column major 4×4 matrix: `m[col * 4 + row]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [f32; 16]);

impl Default for Mat4 {
    fn default() -> Self {
        Mat4::identity()
    }
}

impl Mat4 {
    /// The identity matrix.
    #[inline]
    pub fn identity() -> Self {
        Mat4([
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, //
        ])
    }

    /// Zero matrix.
    #[inline]
    pub fn zero() -> Self {
        Mat4([0.0; 16])
    }

    /// Element access, `row`/`col` in `0..4`.
    #[inline]
    pub fn get(&self, row: usize, col: usize) -> f32 {
        self.0[col * 4 + row]
    }

    /// Element mutation, `row`/`col` in `0..4`.
    #[inline]
    pub fn set(&mut self, row: usize, col: usize, v: f32) {
        self.0[col * 4 + row] = v;
    }

    /// Borrow as a 16 element slice ready for `glUniformMatrix4fv`.
    #[inline]
    pub fn as_slice(&self) -> &[f32; 16] {
        &self.0
    }

    /// Symmetric perspective projection.
    ///
    /// `fovy_deg` is the *vertical* field of view in degrees, `aspect` is
    /// `width / height`.
    pub fn perspective(fovy_deg: f32, aspect: f32, near: f32, far: f32) -> Self {
        let aspect = if aspect.abs() < 1e-6 || !aspect.is_finite() {
            1.0
        } else {
            aspect
        };
        let f = 1.0 / (fovy_deg.to_rad_half().tan());
        let nf = 1.0 / (near - far);
        Mat4([
            f / aspect,
            0.0,
            0.0,
            0.0, //
            0.0,
            f,
            0.0,
            0.0, //
            0.0,
            0.0,
            (far + near) * nf,
            -1.0, //
            0.0,
            0.0,
            2.0 * far * near * nf,
            0.0, //
        ])
    }

    /// Right handed `lookAt`.
    pub fn look_at(eye: [f32; 3], center: [f32; 3], up: [f32; 3]) -> Self {
        let (ex, ey, ez) = (eye[0], eye[1], eye[2]);
        if (ex - center[0]).abs() < 1e-6
            && (ey - center[1]).abs() < 1e-6
            && (ez - center[2]).abs() < 1e-6
        {
            return Mat4::identity();
        }
        let mut z = [ex - center[0], ey - center[1], ez - center[2]];
        normalize(&mut z);
        let mut x = cross(up, z);
        let len = norm(x);
        if len == 0.0 {
            x = [0.0, 0.0, 0.0];
        } else {
            normalize(&mut x);
        }
        let y = cross(z, x);
        Mat4([
            x[0],
            y[0],
            z[0],
            0.0, //
            x[1],
            y[1],
            z[1],
            0.0, //
            x[2],
            y[2],
            z[2],
            0.0, //
            -dot(x, eye),
            -dot(y, eye),
            -dot(z, eye),
            1.0, //
        ])
    }

    /// `self = self · Rx(rad)`.
    pub fn rotate_x(&mut self, rad: f32) {
        let (s, c) = (rad.sin(), rad.cos());
        let a1 = [self.0[4], self.0[5], self.0[6], self.0[7]];
        let a2 = [self.0[8], self.0[9], self.0[10], self.0[11]];
        for i in 0..4 {
            self.0[4 + i] = a1[i] * c + a2[i] * s;
            self.0[8 + i] = a2[i] * c - a1[i] * s;
        }
    }

    /// `self = self · Ry(rad)`.
    pub fn rotate_y(&mut self, rad: f32) {
        let (s, c) = (rad.sin(), rad.cos());
        let a0 = [self.0[0], self.0[1], self.0[2], self.0[3]];
        let a2 = [self.0[8], self.0[9], self.0[10], self.0[11]];
        for i in 0..4 {
            self.0[i] = a0[i] * c - a2[i] * s;
            self.0[8 + i] = a0[i] * s + a2[i] * c;
        }
    }

    /// `self = self · Rz(rad)`.
    pub fn rotate_z(&mut self, rad: f32) {
        let (s, c) = (rad.sin(), rad.cos());
        let a0 = [self.0[0], self.0[1], self.0[2], self.0[3]];
        let a1 = [self.0[4], self.0[5], self.0[6], self.0[7]];
        for i in 0..4 {
            self.0[i] = a0[i] * c + a1[i] * s;
            self.0[4 + i] = a1[i] * c - a0[i] * s;
        }
    }

    /// `self · other`.
    pub fn mul(&self, other: &Mat4) -> Mat4 {
        let a = &self.0;
        let b = &other.0;
        let mut out = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                // out[row, col] = Σ_k a[row, k] · b[k, col]
                let acc = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
                out[col * 4 + row] = acc;
            }
        }
        Mat4(out)
    }

    /// Transform a vec4.
    pub fn transform(&self, v: [f32; 4]) -> [f32; 4] {
        let m = &self.0;
        let mut o = [0.0f32; 4];
        for row in 0..4 {
            o[row] = m[row] * v[0] + m[4 + row] * v[1] + m[8 + row] * v[2] + m[12 + row] * v[3];
        }
        o
    }

    /// Build a GL matrix from a row major 3×3 rotation (as produced by the head
    /// tracker): `matrix[4 * c + r] = so3[r][c]`, last row/column = identity.
    pub fn from_so3_rowmajor(so3: &[[f64; 3]; 3]) -> Self {
        let mut m = Mat4::identity();
        for (r, row) in so3.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                m.0[4 * c + r] = *v as f32;
            }
        }
        m.0[3] = 0.0;
        m.0[7] = 0.0;
        m.0[11] = 0.0;
        m.0[12] = 0.0;
        m.0[13] = 0.0;
        m.0[14] = 0.0;
        m.0[15] = 1.0;
        m
    }

    /// Transpose.
    pub fn transpose(&self) -> Self {
        let mut o = [0.0f32; 16];
        for (idx, v) in o.iter_mut().enumerate() {
            let c = idx / 4;
            let r = idx % 4;
            *v = self.0[r * 4 + c];
        }
        Mat4(o)
    }

    /// Rotation matrix from Euler angles in degrees, following Android's
    /// `Matrix.setRotateEulerM` convention (`R = Rx(x) · Ry(y) · Rz(z)`), stored
    /// column major so it can be uploaded with `transpose = GL_FALSE`.
    ///
    /// Note: the transcription of this formula in the reference player
    /// (`xl_mat4.c`) swaps `cxsy`/`sxsy` and produces a singular matrix for
    /// single axis rotations; the constants used by its head tracker are hard
    /// coded, so the bug is dormant there.  Here the formula is implemented
    /// correctly and unit tested against `Rx`/`Ry`.
    pub fn rotate_euler_deg(x_deg: f32, y_deg: f32, z_deg: f32) -> Self {
        let (x, y, z) = (x_deg.to_rad(), y_deg.to_rad(), z_deg.to_rad());
        let (cx, sx) = (x.cos(), x.sin());
        let (cy, sy) = (y.cos(), y.sin());
        let (cz, sz) = (z.cos(), z.sin());
        let cxsy = cx * sy;
        let sxsy = sx * sy;

        // row major mathematical rotation
        let r = [
            [cy * cz, -cy * sz, sy],
            [sxsy * cz + cx * sz, -sxsy * sz + cx * cz, -sx * cy],
            [-cxsy * cz + sx * sz, cxsy * sz + sx * cz, cx * cy],
        ];
        let mut m = Mat4::identity();
        for (row, values) in r.iter().enumerate() {
            for (col, v) in values.iter().enumerate() {
                m.set(row, col, *v);
            }
        }
        m
    }

    /// Rotation about the Y axis (degrees), as a GL matrix.
    pub fn rotation_y_deg(deg: f32) -> Self {
        let mut m = Mat4::identity();
        m.rotate_y(deg.to_rad());
        m
    }

    /// Rotation about the X axis (degrees), as a GL matrix.
    pub fn rotation_x_deg(deg: f32) -> Self {
        let mut m = Mat4::identity();
        m.rotate_x(deg.to_rad());
        m
    }
}

/// Helper trait to keep the math readable.
trait AngleExt {
    fn to_rad(self) -> f32;
    fn to_rad_half(self) -> f32;
}

impl AngleExt for f32 {
    #[inline]
    fn to_rad(self) -> f32 {
        self * std::f32::consts::PI / 180.0
    }
    #[inline]
    fn to_rad_half(self) -> f32 {
        self * std::f32::consts::PI / 360.0
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn normalize(a: &mut [f32; 3]) {
    let l = norm(*a);
    if l > 0.0 {
        let inv = 1.0 / l;
        a[0] *= inv;
        a[1] *= inv;
        a[2] *= inv;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: &Mat4, b: &Mat4) -> bool {
        a.0.iter()
            .zip(b.0.iter())
            .all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn identity_mul_is_noop() {
        let p = Mat4::perspective(60.0, 16.0 / 9.0, 0.01, 10.0);
        assert!(approx_eq(&Mat4::identity().mul(&p), &p));
        assert!(approx_eq(&p.mul(&Mat4::identity()), &p));
    }

    #[test]
    fn perspective_maps_center_to_center() {
        let p = Mat4::perspective(90.0, 1.0, 0.1, 100.0);
        // A point straight in front of the camera projects to NDC x = y = 0.
        let v = p.transform([0.0, 0.0, -10.0, 1.0]);
        assert!((v[0] / v[3]).abs() < 1e-5);
        assert!((v[1] / v[3]).abs() < 1e-5);
        assert!(v[3] > 0.0, "point in front of camera must have w > 0");
    }

    #[test]
    fn perspective_fov_is_respected() {
        // With fovy = 90 and aspect = 1, a point at 45° above the axis lands
        // exactly on the top edge of the frustum (NDC y = 1).
        let p = Mat4::perspective(90.0, 1.0, 0.1, 100.0);
        let v = p.transform([0.0, 10.0, -10.0, 1.0]);
        assert!((v[1] / v[3] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn look_at_default_is_translation_free() {
        let v = Mat4::look_at([0.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);
        assert!(approx_eq(&v, &Mat4::identity()));
    }

    #[test]
    fn rotate_y_moves_front_point_left() {
        // Reference behaviour: rotating the *model* by +θ about Y moves the
        // point that faces the camera towards -X (screen left).
        let mut m = Mat4::identity();
        m.rotate_y(30f32.to_radians());
        let p = m.transform([0.0, 0.0, -1.0, 1.0]);
        assert!(p[0] < -0.4 && p[0] > -0.6, "x = {}", p[0]);
        assert!(p[2] < 0.0);
    }

    #[test]
    fn rotate_x_moves_front_point_up() {
        let mut m = Mat4::identity();
        m.rotate_x(30f32.to_radians());
        let p = m.transform([0.0, 0.0, -1.0, 1.0]);
        assert!(p[1] > 0.4 && p[1] < 0.6, "y = {}", p[1]);
    }

    #[test]
    fn mul_matches_reference_order() {
        // mul(a, b) == a·b: apply b first, then a.
        let mut a = Mat4::identity();
        a.rotate_y(90f32.to_radians());
        let mut b = Mat4::identity();
        b.rotate_x(90f32.to_radians());
        let ab = a.mul(&b);
        let v = ab.transform([0.0, 0.0, -1.0, 1.0]);
        let v2 = a.transform(b.transform([0.0, 0.0, -1.0, 1.0]));
        for i in 0..4 {
            assert!((v[i] - v2[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn euler_matches_pure_axis_rotations() {
        // R = Rx·Ry·Rz; for a single axis it must equal the elementary rotation.
        let ry = Mat4::rotate_euler_deg(0.0, -90.0, 0.0);
        let expect = Mat4::rotation_y_deg(-90.0);
        assert!(approx_eq(&ry, &expect), "ry = {:?} vs {:?}", ry.0, expect.0);
        let rx = Mat4::rotate_euler_deg(90.0, 0.0, 0.0);
        assert!(approx_eq(&rx, &Mat4::rotation_x_deg(90.0)));
        // must be a proper rotation, unlike the reference transcription
        let det3 = |m: &Mat4| {
            m.get(0, 0) * (m.get(1, 1) * m.get(2, 2) - m.get(2, 1) * m.get(1, 2))
                - m.get(0, 1) * (m.get(1, 0) * m.get(2, 2) - m.get(1, 2) * m.get(2, 0))
                + m.get(0, 2) * (m.get(1, 0) * m.get(2, 1) - m.get(1, 1) * m.get(2, 0))
        };
        for e in [
            Mat4::rotate_euler_deg(0.0, -90.0, 0.0),
            Mat4::rotate_euler_deg(13.0, -42.0, 77.0),
        ] {
            assert!((det3(&e) - 1.0).abs() < 1e-4, "not a rotation: {:?}", e.0);
        }
    }

    #[test]
    fn so3_conversion_is_column_major() {
        let so3 = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 10.0]];
        let m = Mat4::from_so3_rowmajor(&so3);
        // matrix[4 * c + r] = so3[r][c]  (column major upload of a row major SO3)
        for (r, row) in so3.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                assert_eq!(m.0[c * 4 + r], *v as f32, "r={r} c={c}");
            }
        }
        assert_eq!(m.0[15], 1.0);
        assert_eq!(m.0[12], 0.0);
    }
}
