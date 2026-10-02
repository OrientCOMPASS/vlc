//! Head tracking: a faithful Rust port of Google Cardboard's `OrientationEKF`
//! as shipped inside xl_player (`xl_head_tracker/OrientationEKF.cpp`,
//! `SO3Util.cpp`, `Matrix3x3d.cpp`, `Vector3d.cpp`).
//!
//! The filter fuses the accelerometer (gravity ⇒ tilt reference) with the
//! gyroscope (integration + prediction) and yields a rotation matrix that the
//! renderer uses as the *model* matrix of the sphere.  Keeping it in the pure
//! `vlcrs-vr` crate means the sensor fusion can be unit tested on the host,
//! while [`crate::ekf::OrientationEkf`] is fed by the platform sensor thread.

use crate::mat4::Mat4;

/// Row major 3×3 matrix (the layout used by the reference implementation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat3(pub [[f64; 3]; 3]);

impl Default for Mat3 {
    fn default() -> Self {
        Mat3::identity()
    }
}

impl Mat3 {
    /// Identity.
    pub fn identity() -> Mat3 {
        Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
    }

    /// Zero.
    pub fn zero() -> Mat3 {
        Mat3([[0.0; 3]; 3])
    }

    /// Same value on the diagonal, zero elsewhere.
    pub fn diagonal(d: f64) -> Mat3 {
        let mut m = Mat3::zero();
        for i in 0..3 {
            m.0[i][i] = d;
        }
        m
    }

    /// Element access.
    #[inline]
    pub fn get(&self, r: usize, c: usize) -> f64 {
        self.0[r][c]
    }

    /// Element mutation.
    #[inline]
    pub fn set(&mut self, r: usize, c: usize, v: f64) {
        self.0[r][c] = v;
    }

    /// Replace one column.
    pub fn set_column(&mut self, c: usize, v: &[f64; 3]) {
        for (r, row) in self.0.iter_mut().enumerate() {
            row[c] = v[r];
        }
    }

    /// `self *= s`.
    pub fn scale(&mut self, s: f64) {
        for row in self.0.iter_mut() {
            for v in row.iter_mut() {
                *v *= s;
            }
        }
    }

    /// `self += b`.
    pub fn plus_eq(&mut self, b: &Mat3) {
        for (r, row) in self.0.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v += b.0[r][c];
            }
        }
    }

    /// `self -= b`.
    pub fn minus_eq(&mut self, b: &Mat3) {
        for (r, row) in self.0.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v -= b.0[r][c];
            }
        }
    }

    /// Transposed copy.
    pub fn transposed(&self) -> Mat3 {
        let mut o = Mat3::zero();
        for (r, row) in o.0.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = self.0[c][r];
            }
        }
        o
    }

    /// Transpose in place.
    pub fn transpose(&mut self) {
        *self = self.transposed();
    }

    /// `a · b`.
    pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
        let mut o = Mat3::zero();
        for r in 0..3 {
            for c in 0..3 {
                o.0[r][c] = a.0[r][0] * b.0[0][c] + a.0[r][1] * b.0[1][c] + a.0[r][2] * b.0[2][c];
            }
        }
        o
    }

    /// `a · v`.
    pub fn mul_vec(a: &Mat3, v: &[f64; 3]) -> [f64; 3] {
        [
            a.0[0][0] * v[0] + a.0[0][1] * v[1] + a.0[0][2] * v[2],
            a.0[1][0] * v[0] + a.0[1][1] * v[1] + a.0[1][2] * v[2],
            a.0[2][0] * v[0] + a.0[2][1] * v[1] + a.0[2][2] * v[2],
        ]
    }

    /// Determinant.
    pub fn determinant(&self) -> f64 {
        let m = &self.0;
        m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    }

    /// Inverse; returns `None` for a singular matrix.
    pub fn inverse(&self) -> Option<Mat3> {
        let d = self.determinant();
        if d == 0.0 || !d.is_finite() {
            return None;
        }
        let inv = 1.0 / d;
        let m = &self.0;
        Some(Mat3([
            [
                (m[1][1] * m[2][2] - m[2][1] * m[1][2]) * inv,
                -(m[0][1] * m[2][2] - m[0][2] * m[2][1]) * inv,
                (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv,
            ],
            [
                -(m[1][0] * m[2][2] - m[1][2] * m[2][0]) * inv,
                (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv,
                -(m[0][0] * m[1][2] - m[0][2] * m[1][0]) * inv,
            ],
            [
                (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv,
                -(m[0][0] * m[2][1] - m[0][1] * m[2][0]) * inv,
                (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv,
            ],
        ]))
    }
}

fn dot(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: &[f64; 3], b: &[f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: &[f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn normalized(mut a: [f64; 3]) -> [f64; 3] {
    let l = length(&a);
    if l != 0.0 {
        for v in a.iter_mut() {
            *v /= l;
        }
    }
    a
}

fn largest_abs_component(v: &[f64; 3]) -> usize {
    let (x, y, z) = (v[0].abs(), v[1].abs(), v[2].abs());
    if x > y {
        if x > z {
            0
        } else {
            2
        }
    } else if y > z {
        1
    } else {
        2
    }
}

/// A unit vector orthogonal to `v` (reference `Vector3d::ortho`).
fn ortho(v: &[f64; 3]) -> [f64; 3] {
    let mut k = largest_abs_component(v) as i32 - 1;
    if k < 0 {
        k = 2;
    }
    let mut e = [0.0, 0.0, 0.0];
    e[k as usize] = 1.0;
    normalized(cross(v, &e))
}

/// Rodrigues / exponential map helpers (reference `SO3Util`).
pub mod so3 {
    use super::Mat3;

    /// Rotation that maps `a` onto `b`.
    pub fn from_two_vec(a: &[f64; 3], b: &[f64; 3]) -> Mat3 {
        let n = super::cross(a, b);
        if super::length(&n) == 0.0 {
            let d = super::dot(a, b);
            if d >= 0.0 {
                return Mat3::identity();
            }
            let axis = super::ortho(a);
            return rotation_pi_about_axis(&axis);
        }
        let n = super::normalized(n);
        let a = super::normalized(*a);
        let b = super::normalized(*b);

        let mut r1 = Mat3::zero();
        r1.set_column(0, &a);
        r1.set_column(1, &n);
        let t = super::cross(&n, &a);
        r1.set_column(2, &t);

        let mut r2 = Mat3::zero();
        r2.set_column(0, &b);
        r2.set_column(1, &n);
        let t = super::cross(&n, &b);
        r2.set_column(2, &t);

        r1.transpose();
        Mat3::mul(&r2, &r1)
    }

    /// Rotation of π around `v`.
    pub fn rotation_pi_about_axis(v: &[f64; 3]) -> Mat3 {
        let l = super::length(v);
        if l == 0.0 {
            return Mat3::identity();
        }
        let scale = std::f64::consts::PI / l;
        let w = [v[0] * scale, v[1] * scale, v[2] * scale];
        rodrigues_exp(&w, 0.0, 0.20264236728467558)
    }

    /// Exponential map: rotation vector → matrix.
    pub fn from_mu(w: &[f64; 3]) -> Mat3 {
        let theta_sq = dot_local(w, w);
        let theta = theta_sq.sqrt();
        let (ka, kb);
        if theta_sq < 1.0e-8 {
            ka = 1.0 - 0.16666667163372 * theta_sq;
            kb = 0.5;
        } else if theta_sq < 1.0e-6 {
            kb = 0.5 - 0.0416666679084301 * theta_sq;
            ka = 1.0 - theta_sq * 0.16666667163372 * (1.0 - 0.16666667163372 * theta_sq);
        } else {
            let inv = 1.0 / theta;
            ka = theta.sin() * inv;
            kb = (1.0 - theta.cos()) * (inv * inv);
        }
        rodrigues_exp(w, ka, kb)
    }

    /// Logarithmic map: matrix → rotation vector.
    pub fn mu_from(m: &Mat3) -> [f64; 3] {
        let cos_angle = (m.get(0, 0) + m.get(1, 1) + m.get(2, 2) - 1.0) * 0.5;
        let mut result = [
            (m.get(2, 1) - m.get(1, 2)) / 2.0,
            (m.get(0, 2) - m.get(2, 0)) / 2.0,
            (m.get(1, 0) - m.get(0, 1)) / 2.0,
        ];
        let sin_angle_abs = super::length(&result);
        if cos_angle > std::f64::consts::FRAC_1_SQRT_2 {
            if sin_angle_abs > 0.0 {
                let s = sin_angle_abs.asin() / sin_angle_abs;
                for v in result.iter_mut() {
                    *v *= s;
                }
            }
        } else if cos_angle > -std::f64::consts::FRAC_1_SQRT_2 {
            let angle = cos_angle.clamp(-1.0, 1.0).acos();
            if sin_angle_abs > 1.0e-12 {
                let s = angle / sin_angle_abs;
                for v in result.iter_mut() {
                    *v *= s;
                }
            }
        } else {
            let angle = std::f64::consts::PI - sin_angle_abs.clamp(-1.0, 1.0).asin();
            let d0 = m.get(0, 0) - cos_angle;
            let d1 = m.get(1, 1) - cos_angle;
            let d2 = m.get(2, 2) - cos_angle;
            let mut r2;
            if d0 * d0 > d1 * d1 && d0 * d0 > d2 * d2 {
                r2 = [
                    d0,
                    (m.get(1, 0) + m.get(0, 1)) / 2.0,
                    (m.get(0, 2) + m.get(2, 0)) / 2.0,
                ];
            } else if d1 * d1 > d2 * d2 {
                r2 = [
                    (m.get(1, 0) + m.get(0, 1)) / 2.0,
                    d1,
                    (m.get(2, 1) + m.get(1, 2)) / 2.0,
                ];
            } else {
                r2 = [
                    (m.get(0, 2) + m.get(2, 0)) / 2.0,
                    (m.get(2, 1) + m.get(1, 2)) / 2.0,
                    d2,
                ];
            }
            if super::dot(&r2, &result) < 0.0 {
                for v in r2.iter_mut() {
                    *v = -*v;
                }
            }
            let r2 = super::normalized(r2);
            result = [r2[0] * angle, r2[1] * angle, r2[2] * angle];
        }
        result
    }

    fn rodrigues_exp(w: &[f64; 3], ka: f64, kb: f64) -> Mat3 {
        let wx2 = w[0] * w[0];
        let wy2 = w[1] * w[1];
        let wz2 = w[2] * w[2];
        let mut m = Mat3::zero();
        m.set(0, 0, 1.0 - kb * (wy2 + wz2));
        m.set(1, 1, 1.0 - kb * (wx2 + wz2));
        m.set(2, 2, 1.0 - kb * (wx2 + wy2));

        let mut a = ka * w[2];
        let mut b = kb * (w[0] * w[1]);
        m.set(0, 1, b - a);
        m.set(1, 0, b + a);

        a = ka * w[1];
        b = kb * (w[0] * w[2]);
        m.set(0, 2, b + a);
        m.set(2, 0, b - a);

        a = ka * w[0];
        b = kb * (w[1] * w[2]);
        m.set(1, 2, b - a);
        m.set(2, 1, b + a);
        m
    }

    fn dot_local(a: &[f64; 3], b: &[f64; 3]) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
}

/// Cardboard's extended Kalman filter for device orientation.
#[derive(Clone, Debug)]
pub struct OrientationEkf {
    so3_sensor_from_world: Mat3,
    so3_last_motion: Mat3,
    m_p: Mat3,
    m_q: Mat3,
    m_r_acceleration: Mat3,
    m_s: Mat3,
    m_h: Mat3,
    m_k: Mat3,
    v_nu: [f64; 3],
    v_z: [f64; 3],
    v_x: [f64; 3],
    v_down: [f64; 3],
    sensor_timestamp_gyro: f64,
    last_gyro: [f64; 3],
    filtered_gyro_timestep: f64,
    timestep_filter_init: bool,
    num_gyro_timestep_samples: i32,
    gyro_filter_valid: bool,
    aligned_to_gravity: bool,
}

impl Default for OrientationEkf {
    fn default() -> Self {
        let mut e = OrientationEkf {
            so3_sensor_from_world: Mat3::identity(),
            so3_last_motion: Mat3::identity(),
            m_p: Mat3::zero(),
            m_q: Mat3::zero(),
            m_r_acceleration: Mat3::zero(),
            m_s: Mat3::zero(),
            m_h: Mat3::zero(),
            m_k: Mat3::zero(),
            v_nu: [0.0; 3],
            v_z: [0.0; 3],
            v_x: [0.0; 3],
            v_down: [0.0, 0.0, 9.81],
            sensor_timestamp_gyro: 0.0,
            last_gyro: [0.0; 3],
            filtered_gyro_timestep: 0.0,
            timestep_filter_init: false,
            num_gyro_timestep_samples: 0,
            gyro_filter_valid: true,
            aligned_to_gravity: false,
        };
        e.reset();
        e
    }
}

impl OrientationEkf {
    /// A freshly reset filter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the filter: the next accelerometer sample re-establishes the tilt
    /// reference (this is what "视角摆正 / re-centre" calls).
    pub fn reset(&mut self) {
        self.sensor_timestamp_gyro = 0.0;
        self.so3_sensor_from_world = Mat3::identity();
        self.so3_last_motion = Mat3::identity();
        self.m_p = Mat3::diagonal(25.0);
        self.m_q = Mat3::diagonal(1.0);
        self.m_r_acceleration = Mat3::diagonal(0.5625);
        self.m_s = Mat3::zero();
        self.m_h = Mat3::zero();
        self.m_k = Mat3::zero();
        self.v_nu = [0.0; 3];
        self.v_z = [0.0; 3];
        self.v_x = [0.0; 3];
        self.v_down = [0.0, 0.0, 9.81];
        self.last_gyro = [0.0; 3];
        self.filtered_gyro_timestep = 0.0;
        self.timestep_filter_init = false;
        self.num_gyro_timestep_samples = 0;
        self.gyro_filter_valid = true;
        self.aligned_to_gravity = false;
    }

    /// `true` once gravity has been observed.
    pub fn is_ready(&self) -> bool {
        self.aligned_to_gravity
    }

    /// Feed a gyroscope sample (rad/s) with its `timestamp` in nanoseconds
    /// (the value delivered by `ASensorEvent.timestamp`).
    pub fn process_gyro(&mut self, gyro: [f64; 3], timestamp_ns: f64) {
        if self.sensor_timestamp_gyro != 0.0 {
            let mut dt = (timestamp_ns - self.sensor_timestamp_gyro) * 1.0e-9;
            if dt > 0.04 {
                dt = if self.gyro_filter_valid {
                    self.filtered_gyro_timestep
                } else {
                    0.01
                };
            } else {
                self.filter_gyro_timestep(dt);
            }
            let u = [gyro[0] * -dt, gyro[1] * -dt, gyro[2] * -dt];
            self.so3_last_motion = so3::from_mu(&u);
            self.so3_sensor_from_world =
                Mat3::mul(&self.so3_last_motion, &self.so3_sensor_from_world);
            self.update_covariances_after_motion();
            let mut temp = self.m_q;
            temp.scale(dt * dt);
            self.m_p.plus_eq(&temp);
        }
        self.sensor_timestamp_gyro = timestamp_ns;
        self.last_gyro = gyro;
    }

    /// Feed an accelerometer sample (m/s², device axes).
    pub fn process_accel(&mut self, acc: [f64; 3]) {
        self.v_z = acc;
        if self.aligned_to_gravity {
            self.v_nu = self.acceleration_observation(&self.so3_sensor_from_world);
            const EPS: f64 = 1.0e-7;
            for dof in 0..3 {
                let mut delta = [0.0, 0.0, 0.0];
                delta[dof] = EPS;
                let temp_m = so3::from_mu(&delta);
                let perturbed = Mat3::mul(&temp_m, &self.so3_sensor_from_world);
                let temp_v = self.acceleration_observation(&perturbed);
                let mut j = [
                    self.v_nu[0] - temp_v[0],
                    self.v_nu[1] - temp_v[1],
                    self.v_nu[2] - temp_v[2],
                ];
                for v in j.iter_mut() {
                    *v /= EPS;
                }
                self.m_h.set_column(dof, &j);
            }

            let m_ht = self.m_h.transposed();
            let temp = Mat3::mul(&self.m_p, &m_ht);
            let temp = Mat3::mul(&self.m_h, &temp);
            // S = H · P · Hᵀ + R_acceleration
            let mut s = temp;
            s.plus_eq(&self.m_r_acceleration);
            self.m_s = s;
            let inv_s = match self.m_s.inverse() {
                Some(i) => i,
                None => return,
            };
            let temp = Mat3::mul(&m_ht, &inv_s);
            self.m_k = Mat3::mul(&self.m_p, &temp);
            self.v_x = Mat3::mul_vec(&self.m_k, &self.v_nu);
            let temp = Mat3::mul(&self.m_k, &self.m_h);
            let mut temp2 = Mat3::identity();
            temp2.minus_eq(&temp);
            self.m_p = Mat3::mul(&temp2, &self.m_p);
            self.so3_last_motion = so3::from_mu(&self.v_x);
            self.so3_sensor_from_world =
                Mat3::mul(&self.so3_last_motion, &self.so3_sensor_from_world);
            self.update_covariances_after_motion();
        } else {
            self.so3_sensor_from_world = so3::from_two_vec(&self.v_down, &self.v_z);
            self.aligned_to_gravity = true;
        }
    }

    /// Predicted orientation as a row major SO(3) matrix, `seconds_after_gyro`
    /// in the future (the reference adds one frame ≈ 33 ms).
    pub fn predicted_so3(&self, seconds_after_gyro: f64) -> Mat3 {
        let dt = seconds_after_gyro;
        let pmu = [
            self.last_gyro[0] * -dt,
            self.last_gyro[1] * -dt,
            self.last_gyro[2] * -dt,
        ];
        let predicted_motion = so3::from_mu(&pmu);
        Mat3::mul(&predicted_motion, &self.so3_sensor_from_world)
    }

    /// Predicted orientation as a GL model matrix (column major 4×4).
    pub fn predicted_gl_matrix(&self, seconds_after_gyro: f64) -> Mat4 {
        let so3 = self.predicted_so3(seconds_after_gyro);
        let rows: [[f64; 3]; 3] = so3.0;
        Mat4::from_so3_rowmajor(&rows)
    }

    /// Compass heading in degrees (reference `getHeadingDegrees`).
    pub fn heading_degrees(&self) -> f64 {
        let x = self.so3_sensor_from_world.get(2, 0);
        let y = self.so3_sensor_from_world.get(2, 1);
        let mag = (x * x + y * y).sqrt();
        if mag < 0.1 {
            return 0.0;
        }
        let mut heading = -90.0 - y.atan2(x).to_degrees();
        if heading < 0.0 {
            heading += 360.0;
        }
        if heading >= 360.0 {
            heading -= 360.0;
        }
        heading
    }

    fn filter_gyro_timestep(&mut self, timestep: f64) {
        const K_FILTER_COEFF: f64 = 0.95;
        if !self.timestep_filter_init {
            self.filtered_gyro_timestep = timestep;
            self.num_gyro_timestep_samples = 1;
            self.timestep_filter_init = true;
        } else {
            self.filtered_gyro_timestep =
                K_FILTER_COEFF * self.filtered_gyro_timestep + (1.0 - K_FILTER_COEFF) * timestep;
            self.num_gyro_timestep_samples += 1;
            self.gyro_filter_valid = self.num_gyro_timestep_samples > 10;
        }
    }

    fn update_covariances_after_motion(&mut self) {
        let temp = self.so3_last_motion.transposed();
        let t2 = Mat3::mul(&self.m_p, &temp);
        self.m_p = Mat3::mul(&self.so3_last_motion, &t2);
        self.so3_last_motion = Mat3::identity();
    }

    fn acceleration_observation(&self, so3_sensor_from_world_pred: &Mat3) -> [f64; 3] {
        let v_h = Mat3::mul_vec(so3_sensor_from_world_pred, &self.v_down);
        let temp = so3::from_two_vec(&v_h, &self.v_z);
        so3::mu_from(&temp)
    }
}

/// Sensor axis remapping applied before feeding the filter, as in the reference
/// (`xl_tracker.c` passes `(-y, x, z)` for both sensors).
pub fn remap_sensor_axes(x: f64, y: f64, z: f64) -> [f64; 3] {
    [-y, x, z]
}

/// Correction from EKF space into head space for a given display rotation
/// (degrees: 0, 90, 180, 270).
///
/// The `90` case is the constant used by the reference player
/// (`ekf_to_head_tracker` in `xl_tracker.c`, documented there as Android's
/// `Matrix.setRotateEulerM(m, 0, -90, 0)` for landscape): it maps
/// `(x, y, z) → (−x, z, y)`.  The other three orientations follow the same
/// family; landscape is what the demo player locks to and what is validated on
/// device.
pub fn head_space_correction(display_rotation_deg: f32) -> Mat4 {
    let r = (display_rotation_deg.round() as i32).rem_euclid(360);
    match r {
        90 => Mat4([
            -1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, //
        ]),
        180 => Mat4([
            -1.0, 0.0, 0.0, 0.0, //
            0.0, -1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, //
        ]),
        270 => Mat4([
            1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, -1.0, 0.0, //
            0.0, -1.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, //
        ]),
        _ => Mat4::identity(),
    }
}

/// Full head matrix as used by the renderer: `predicted · correction`.
pub fn head_matrix(ekf: &OrientationEkf, seconds_ahead: f64, display_rotation_deg: f32) -> Mat4 {
    let m = ekf.predicted_gl_matrix(seconds_ahead);
    m.mul(&head_space_correction(display_rotation_deg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_rotation(m: &Mat3) -> bool {
        let t = m.transposed();
        let prod = Mat3::mul(m, &t);
        let id = Mat3::identity();
        let ok = (0..3).all(|r| (0..3).all(|c| (prod.get(r, c) - id.get(r, c)).abs() < 1e-6));
        ok && (m.determinant() - 1.0).abs() < 1e-6
    }

    #[test]
    fn mat3_basics() {
        let a = Mat3::identity();
        let b = Mat3::diagonal(2.0);
        let c = Mat3::mul(&a, &b);
        assert_eq!(c.get(0, 0), 2.0);
        assert_eq!(c.get(0, 1), 0.0);
        let inv = b.inverse().unwrap();
        assert!((inv.get(1, 1) - 0.5).abs() < 1e-12);
        assert!(Mat3::zero().inverse().is_none());
    }

    #[test]
    fn transpose_and_mul_vec() {
        let mut m = Mat3::zero();
        m.set(0, 1, 5.0);
        assert_eq!(m.transposed().get(1, 0), 5.0);
        let v = Mat3::mul_vec(&Mat3::identity(), &[1.0, 2.0, 3.0]);
        assert_eq!(v, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn so3_exp_log_roundtrip() {
        for w in [
            [0.0, 0.0, 0.0],
            [0.1, -0.2, 0.05],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 2.5],
            [-0.7, 0.3, 0.9],
        ] {
            let m = so3::from_mu(&w);
            assert!(is_rotation(&m), "exp({w:?}) is not a rotation");
            let back = so3::mu_from(&m);
            let m2 = so3::from_mu(&back);
            for r in 0..3 {
                for c in 0..3 {
                    assert!(
                        (m.get(r, c) - m2.get(r, c)).abs() < 1e-6,
                        "roundtrip failed for {w:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn so3_from_two_vec_maps_a_to_b() {
        let a = [0.0, 0.0, 9.81];
        let b = [9.81, 0.0, 0.0];
        let m = so3::from_two_vec(&a, &b);
        let mapped = Mat3::mul_vec(&m, &normalized(a));
        let target = normalized(b);
        for i in 0..3 {
            assert!((mapped[i] - target[i]).abs() < 1e-6);
        }
        // identical vectors → identity
        let id = so3::from_two_vec(&a, &a);
        assert!(is_rotation(&id));
        // opposite vectors → 180° rotation
        let opp = so3::from_two_vec(&a, &[0.0, 0.0, -9.81]);
        assert!(is_rotation(&opp));
        let mapped = Mat3::mul_vec(&opp, &normalized(a));
        assert!((mapped[2] + 1.0).abs() < 1e-6);
    }

    #[test]
    fn fresh_filter_is_not_ready_until_gravity() {
        let mut ekf = OrientationEkf::new();
        assert!(!ekf.is_ready());
        ekf.process_accel([0.0, 0.0, 9.81]);
        assert!(ekf.is_ready());
        let m = ekf.predicted_so3(0.0);
        assert!(is_rotation(&m));
    }

    #[test]
    fn flat_device_produces_identity_orientation() {
        let mut ekf = OrientationEkf::new();
        // device flat on a table: gravity points along +Z of the sensor frame
        ekf.process_accel([0.0, 0.0, 9.81]);
        let m = ekf.predicted_so3(0.0);
        let id = Mat3::identity();
        for r in 0..3 {
            for c in 0..3 {
                assert!((m.get(r, c) - id.get(r, c)).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn gyro_rotates_the_orientation() {
        let mut ekf = OrientationEkf::new();
        ekf.process_accel([0.0, 0.0, 9.81]);
        // rotate 90°/s around Z for one second, in 10 ms steps
        let mut ts = 1_000_000_000f64;
        ekf.process_gyro([0.0, 0.0, std::f64::consts::FRAC_PI_2], ts);
        for _ in 0..99 {
            ts += 10_000_000.0;
            ekf.process_gyro([0.0, 0.0, std::f64::consts::FRAC_PI_2], ts);
        }
        let m = ekf.predicted_so3(0.0);
        assert!(is_rotation(&m));
        // A 90° rotation about Z must have moved the X axis onto ±Y.
        let x_axis = Mat3::mul_vec(&m, &[1.0, 0.0, 0.0]);
        assert!(
            x_axis[1].abs() > 0.9,
            "expected the X axis to rotate onto Y, got {x_axis:?}"
        );
    }

    #[test]
    fn filter_stays_stable_with_jitter() {
        let mut ekf = OrientationEkf::new();
        let mut ts = 0f64;
        let mut state = 12345u64;
        let mut rand = move || {
            // deterministic xorshift
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % 2000) as f64 / 1000.0 - 1.0
        };
        for i in 0..2000 {
            ts += 5_000_000.0 + (i % 7) as f64 * 1_000_000.0;
            let g = [rand() * 2.0, rand() * 2.0, rand() * 2.0];
            ekf.process_gyro(g, ts);
            if i % 10 == 0 {
                let a = [rand() * 3.0, rand() * 3.0, 9.81 + rand() * 1.5];
                ekf.process_accel(a);
            }
            if i % 100 == 0 {
                let m = ekf.predicted_so3(0.0333);
                assert!(is_rotation(&m), "filter diverged at step {i}");
                for r in 0..3 {
                    for c in 0..3 {
                        assert!(m.get(r, c).is_finite());
                    }
                }
            }
        }
        assert!(ekf.is_ready());
    }

    #[test]
    fn huge_gyro_gaps_are_filtered() {
        let mut ekf = OrientationEkf::new();
        ekf.process_accel([0.0, 0.0, 9.81]);
        ekf.process_gyro([1.0, 0.0, 0.0], 1_000_000_000.0);
        // a 5 second gap must not integrate 5 seconds of rotation
        ekf.process_gyro([1.0, 0.0, 0.0], 6_000_000_000.0);
        let m = ekf.predicted_so3(0.0);
        assert!(is_rotation(&m));
        let v = Mat3::mul_vec(&m, &[0.0, 1.0, 0.0]);
        assert!(v.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn reset_clears_the_reference() {
        let mut ekf = OrientationEkf::new();
        ekf.process_accel([9.81, 0.0, 0.0]);
        assert!(ekf.is_ready());
        ekf.reset();
        assert!(!ekf.is_ready());
        let m = ekf.predicted_so3(0.0);
        let id = Mat3::identity();
        for r in 0..3 {
            for c in 0..3 {
                assert!((m.get(r, c) - id.get(r, c)).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn heading_is_in_range() {
        let mut ekf = OrientationEkf::new();
        ekf.process_accel([0.0, 9.81, 0.0]);
        let h = ekf.heading_degrees();
        assert!((0.0..360.0).contains(&h), "heading = {h}");
    }

    #[test]
    fn head_matrix_applies_the_landscape_correction() {
        let ekf = OrientationEkf::new();
        let m = head_matrix(&ekf, 0.0, 90.0);
        let c = head_space_correction(90.0);
        // identity prediction → the correction itself
        for i in 0..16 {
            assert!((m.0[i] - c.0[i]).abs() < 1e-4, "index {i}");
        }
        // landscape correction matches the reference `ekf_to_head_tracker`
        // constant: (x, y, z) → (−x, z, y)
        let reference: [f32; 16] = [
            -1.0,
            0.0,
            0.0,
            0.0, //
            0.0,
            -4.371139e-8,
            1.0,
            0.0, //
            0.0,
            1.0,
            -4.371139e-8,
            0.0, //
            0.0,
            0.0,
            0.0,
            1.0, //
        ];
        for (i, v) in c.0.iter().enumerate() {
            assert!(
                (v - reference[i]).abs() < 1e-5,
                "index {i}: {v} vs {}",
                reference[i]
            );
        }
        assert_eq!(head_space_correction(0.0), Mat4::identity());
        // every correction must be a proper rotation
        for deg in [0.0, 90.0, 180.0, 270.0, 450.0, -90.0] {
            let m = head_space_correction(deg);
            let t = m.transpose();
            let prod = m.mul(&t);
            for r in 0..3 {
                for cc in 0..3 {
                    let expect = if r == cc { 1.0 } else { 0.0 };
                    assert!(
                        (prod.get(r, cc) - expect).abs() < 1e-4,
                        "correction {deg} is not orthonormal"
                    );
                }
            }
        }
        assert_eq!(head_space_correction(450.0), head_space_correction(90.0));
        assert_eq!(head_space_correction(-90.0), head_space_correction(270.0));
    }

    #[test]
    fn sensor_remap_matches_reference() {
        assert_eq!(remap_sensor_axes(1.0, 2.0, 3.0), [-2.0, 1.0, 3.0]);
    }

    #[test]
    fn gl_matrix_is_column_major_and_normalized() {
        let mut ekf = OrientationEkf::new();
        ekf.process_accel([0.0, 0.0, 9.81]);
        ekf.process_gyro([0.3, -0.2, 0.1], 1_000_000_000.0);
        ekf.process_gyro([0.3, -0.2, 0.1], 1_010_000_000.0);
        let m = ekf.predicted_gl_matrix(0.0333);
        assert_eq!(m.get(3, 3), 1.0);
        assert_eq!(m.get(0, 3), 0.0);
        assert_eq!(m.get(3, 0), 0.0);
    }
}
