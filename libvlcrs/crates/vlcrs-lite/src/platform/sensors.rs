//! Head tracking thread: accelerometer + gyroscope through `ASensorManager`,
//! fused by [`vlcrs_vr::ekf::OrientationEkf`].
//!
//! The reference player runs the same pipeline (`xl_tracker.c`): a dedicated
//! thread owning an `ALooper`/`ASensorEventQueue`, feeding the filter with the
//! axes remapped to `(-y, x, z)`, and publishing a predicted orientation.
//!
//! One deviation from the reference on purpose: it computes the prediction
//! horizon by mixing `gettimeofday()` with `ASensorEvent.timestamp` (which is
//! boot relative), so the horizon explodes and the prediction degenerates.
//! Here the horizon is a fixed one frame, which is what it is meant to
//! compensate for anyway.

use std::ffi::CString;
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use vlcrs_vr::ekf::{head_matrix, remap_sensor_axes, OrientationEkf};
use vlcrs_vr::mat4::Mat4;

use super::ndk::*;

/// Sensor sampling period requested from the platform (µs).
const SENSOR_RATE_US: i32 = 10_000;
/// Prediction horizon: one frame at 30 fps.
const PREDICT_SECONDS: f64 = 0.033_333_333_333_333_33;
/// Looper ident used for the sensor queue.
const LOOPER_ID: c_int = 3;
/// Looper poll timeout, so the thread notices shutdown quickly.
const POLL_TIMEOUT_MS: c_int = 100;

struct Shared {
    ekf: Mutex<OrientationEkf>,
    last_gyro_ns: AtomicI64,
    samples: AtomicI64,
    running: AtomicBool,
    ready: AtomicBool,
    looper: Mutex<*mut c_void>,
}

// The looper pointer is only dereferenced by the sensor thread itself; other
// threads may only hand it to `ALooper_wake`, which is thread safe.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

/// A running head tracker.
pub struct HeadTracker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    display_rotation: f32,
}

impl HeadTracker {
    /// Start tracking.  `package` is the application package name (required by
    /// `ASensorManager_getInstanceForPackage`).
    pub fn start(package: &str) -> Option<HeadTracker> {
        let shared = Arc::new(Shared {
            ekf: Mutex::new(OrientationEkf::new()),
            last_gyro_ns: AtomicI64::new(0),
            samples: AtomicI64::new(0),
            running: AtomicBool::new(true),
            ready: AtomicBool::new(false),
            looper: Mutex::new(ptr::null_mut()),
        });
        let pkg = CString::new(package).ok()?;
        let thread_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("vlcrs-tracker".into())
            .spawn(move || sensor_thread(thread_shared, pkg))
            .ok()?;
        Some(HeadTracker {
            shared,
            thread: Some(handle),
            display_rotation: 90.0,
        })
    }

    /// Set the display rotation used to convert sensor space into head space
    /// (0/90/180/270 degrees).
    pub fn set_display_rotation(&mut self, deg: f32) {
        self.display_rotation = deg;
    }

    /// Display rotation currently in use.
    pub fn display_rotation(&self) -> f32 {
        self.display_rotation
    }

    /// `true` once gravity has been observed and the filter is usable.
    pub fn is_ready(&self) -> bool {
        self.shared.ready.load(Ordering::Relaxed)
    }

    /// Number of sensor samples processed (diagnostics).
    pub fn samples(&self) -> i64 {
        self.shared.samples.load(Ordering::Relaxed)
    }

    /// Timestamp of the last gyroscope sample, in nanoseconds (sensor clock).
    pub fn last_gyro_ns(&self) -> i64 {
        self.shared.last_gyro_ns.load(Ordering::Relaxed)
    }

    /// Predicted head orientation as a GL model matrix, or `None` when the
    /// filter has no gravity reference yet.
    pub fn head_matrix(&self) -> Option<Mat4> {
        if !self.is_ready() {
            return None;
        }
        let ekf = self.shared.ekf.lock().ok()?;
        Some(head_matrix(&ekf, PREDICT_SECONDS, self.display_rotation))
    }

    /// Re-align the reference orientation ("视角摆正").
    pub fn reset(&self) {
        if let Ok(mut ekf) = self.shared.ekf.lock() {
            ekf.reset();
        }
        self.shared.ready.store(false, Ordering::Relaxed);
    }

    /// Stop the tracker thread.
    pub fn stop(&mut self) {
        self.shared.running.store(false, Ordering::Relaxed);
        if let Ok(l) = self.shared.looper.lock() {
            if !l.is_null() {
                unsafe { ALooper_wake(*l) };
            }
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for HeadTracker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn sensor_thread(shared: Arc<Shared>, package: CString) {
    crate::platform::media::set_thread_name("vlcrs-track");
    let mgr = unsafe { ASensorManager_getInstanceForPackage(package.as_ptr()) };
    if mgr.is_null() {
        crate::vwarn!("ASensorManager unavailable: head tracking disabled");
        shared.running.store(false, Ordering::Relaxed);
        return;
    }
    let acc = unsafe { ASensorManager_getDefaultSensor(mgr, sensor_type::ACCELEROMETER) };
    let gyro = unsafe { ASensorManager_getDefaultSensor(mgr, sensor_type::GYROSCOPE) };
    if acc.is_null() || gyro.is_null() {
        crate::vwarn!("no accelerometer/gyroscope on this device: head tracking disabled");
        shared.running.store(false, Ordering::Relaxed);
        return;
    }
    let looper = unsafe { ALooper_prepare(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS) };
    if looper.is_null() {
        crate::verror!("ALooper_prepare failed");
        shared.running.store(false, Ordering::Relaxed);
        return;
    }
    if let Ok(mut l) = shared.looper.lock() {
        *l = looper;
    }
    let queue = unsafe {
        ASensorManager_createEventQueue(mgr, looper, LOOPER_ID, ptr::null_mut(), ptr::null_mut())
    };
    if queue.is_null() {
        crate::verror!("ASensorManager_createEventQueue failed");
        shared.running.store(false, Ordering::Relaxed);
        return;
    }
    unsafe {
        ASensorEventQueue_enableSensor(queue, acc);
        ASensorEventQueue_enableSensor(queue, gyro);
        let acc_delay = SENSOR_RATE_US.max(ASensor_getMinDelay(acc));
        let gyro_delay = SENSOR_RATE_US.max(ASensor_getMinDelay(gyro));
        ASensorEventQueue_setEventRate(queue, acc, acc_delay);
        ASensorEventQueue_setEventRate(queue, gyro, gyro_delay);
    }
    crate::vlog!("head tracker started (accelerometer + gyroscope @ {SENSOR_RATE_US}us)");

    let mut events = [ASensorEvent::default(); 8];
    while shared.running.load(Ordering::Relaxed) {
        let ident = unsafe {
            ALooper_pollOnce(
                POLL_TIMEOUT_MS,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if ident == ALOOPER_POLL_ERROR {
            break;
        }
        if ident != LOOPER_ID {
            continue;
        }
        loop {
            let n =
                unsafe { ASensorEventQueue_getEvents(queue, events.as_mut_ptr(), events.len()) };
            if n <= 0 {
                break;
            }
            for ev in events.iter().take(n as usize) {
                let axes = remap_sensor_axes(
                    f64::from(ev.data[0]),
                    f64::from(ev.data[1]),
                    f64::from(ev.data[2]),
                );
                let Ok(mut ekf) = shared.ekf.lock() else {
                    continue;
                };
                match ev.sensor_type {
                    sensor_type::ACCELEROMETER => {
                        ekf.process_accel(axes);
                        shared.ready.store(ekf.is_ready(), Ordering::Relaxed);
                    }
                    sensor_type::GYROSCOPE => {
                        ekf.process_gyro(axes, ev.timestamp as f64);
                        shared.last_gyro_ns.store(ev.timestamp, Ordering::Relaxed);
                    }
                    _ => {}
                }
                shared.samples.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    unsafe {
        ASensorEventQueue_disableSensor(queue, acc);
        ASensorEventQueue_disableSensor(queue, gyro);
        ASensorManager_destroyEventQueue(mgr, queue);
        ALooper_release(looper);
    }
    if let Ok(mut l) = shared.looper.lock() {
        *l = ptr::null_mut();
    }
    crate::vlog!("head tracker stopped");
}
