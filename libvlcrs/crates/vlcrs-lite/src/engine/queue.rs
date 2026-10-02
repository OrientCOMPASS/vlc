//! Bounded, epoch aware packet queue shared by the demux thread and the decode
//! threads.
//!
//! Every packet carries the *epoch* it was read in; a seek bumps the epoch and
//! flushes the queue, so a consumer that is mid-flight simply discards stale
//! packets instead of needing a lock on the codec.

// The engine is Android-only; on host builds these helpers exist for the unit
// tests below.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub(crate) struct Packet {
    pub data: Vec<u8>,
    pub pts_us: i64,
    pub flags: u32,
    pub eos: bool,
    pub epoch: u64,
}

struct QState {
    q: VecDeque<Packet>,
    bytes: usize,
    aborted: bool,
    /// number of flushes performed, used to wake pushers
    generation: u64,
}

pub(crate) struct PacketQueue {
    state: Mutex<QState>,
    not_empty: Condvar,
    not_full: Condvar,
    max_bytes: usize,
    max_packets: usize,
    dropped: AtomicU64,
}

impl PacketQueue {
    pub(crate) fn new(max_bytes: usize, max_packets: usize) -> PacketQueue {
        PacketQueue {
            state: Mutex::new(QState {
                q: VecDeque::new(),
                bytes: 0,
                aborted: false,
                generation: 0,
            }),
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
            max_bytes,
            max_packets,
            dropped: AtomicU64::new(0),
        }
    }

    /// Push a packet, waiting at most `timeout` for room.
    /// Returns `false` when the queue is aborted or the timeout expired.
    pub(crate) fn push(&self, packet: Packet, timeout: Duration) -> bool {
        let Ok(mut st) = self.state.lock() else {
            return false;
        };
        let deadline = Instant::now() + timeout;
        let size = packet.data.len();
        while !st.aborted
            && (st.q.len() >= self.max_packets || st.bytes + size > self.max_bytes)
            && !st.q.iter().any(|p| p.eos)
        {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let (guard, _res) = match self.not_full.wait_timeout(st, deadline - now) {
                Ok(v) => v,
                Err(_) => return false,
            };
            st = guard;
        }
        if st.aborted {
            return false;
        }
        st.bytes += size;
        st.q.push_back(packet);
        self.not_empty.notify_one();
        true
    }

    /// Pop the next packet, waiting at most `timeout`.
    pub(crate) fn pop(&self, timeout: Duration) -> Option<Packet> {
        let mut st = self.state.lock().ok()?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(p) = st.q.pop_front() {
                st.bytes = st.bytes.saturating_sub(p.data.len());
                drop(st);
                self.not_full.notify_one();
                return Some(p);
            }
            if st.aborted {
                return None;
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, _res) = self.not_empty.wait_timeout(st, deadline - now).ok()?;
            st = guard;
        }
    }

    /// Drop every queued packet (seek/flush).
    pub(crate) fn flush(&self) {
        if let Ok(mut st) = self.state.lock() {
            st.bytes = 0;
            st.q.clear();
            st.generation += 1;
            self.not_empty.notify_all();
            self.not_full.notify_all();
        }
    }

    /// Wake everybody and refuse further traffic until [`PacketQueue::reset`].
    pub(crate) fn abort(&self) {
        if let Ok(mut st) = self.state.lock() {
            st.aborted = true;
            st.bytes = 0;
            st.q.clear();
            self.not_empty.notify_all();
            self.not_full.notify_all();
        }
    }

    /// Re-arm an aborted queue.
    pub(crate) fn reset(&self) {
        if let Ok(mut st) = self.state.lock() {
            st.aborted = false;
            st.bytes = 0;
            st.q.clear();
            st.generation += 1;
        }
    }

    #[allow(dead_code)] // diagnostics, exercised by the unit tests
    pub(crate) fn len(&self) -> usize {
        self.state.lock().map(|s| s.q.len()).unwrap_or(0)
    }

    #[allow(dead_code)] // diagnostics, exercised by the unit tests
    pub(crate) fn bytes(&self) -> usize {
        self.state.lock().map(|s| s.bytes).unwrap_or(0)
    }

    pub(crate) fn is_aborted(&self) -> bool {
        self.state.lock().map(|s| s.aborted).unwrap_or(true)
    }

    /// Buffered media duration in milliseconds (from the queued PTS span).
    pub(crate) fn buffered_ms(&self) -> i64 {
        let Ok(st) = self.state.lock() else {
            return 0;
        };
        match (st.q.front(), st.q.back()) {
            (Some(a), Some(b)) if b.pts_us > a.pts_us => (b.pts_us - a.pts_us) / 1000,
            _ => 0,
        }
    }

    /// Drop stale packets (epoch mismatch) and count them.
    pub(crate) fn drop_stale(&self, epoch: u64) -> usize {
        let Ok(mut st) = self.state.lock() else {
            return 0;
        };
        let before = st.q.len();
        while let Some(front) = st.q.front() {
            if front.epoch != epoch {
                let p = st.q.pop_front().expect("just peeked");
                st.bytes = st.bytes.saturating_sub(p.data.len());
            } else {
                break;
            }
        }
        let removed = before - st.q.len();
        if removed > 0 {
            self.dropped.fetch_add(removed as u64, Ordering::Relaxed);
            self.not_full.notify_all();
        }
        removed
    }

    #[allow(dead_code)] // diagnostics, exercised by the unit tests
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(n: u8, epoch: u64) -> Packet {
        Packet {
            data: vec![n; 10],
            pts_us: i64::from(n) * 1000,
            flags: 0,
            eos: false,
            epoch,
        }
    }

    #[test]
    fn push_pop_roundtrip() {
        let q = PacketQueue::new(1024, 4);
        assert!(q.push(pkt(1, 0), Duration::from_millis(10)));
        assert!(q.push(pkt(2, 0), Duration::from_millis(10)));
        assert_eq!(q.len(), 2);
        assert_eq!(q.bytes(), 20);
        let p = q.pop(Duration::from_millis(10)).unwrap();
        assert_eq!(p.data[0], 1);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn pop_times_out_when_empty() {
        let q = PacketQueue::new(1024, 4);
        let t = Instant::now();
        assert!(q.pop(Duration::from_millis(30)).is_none());
        assert!(t.elapsed() >= Duration::from_millis(25));
    }

    #[test]
    fn respects_the_packet_cap() {
        let q = PacketQueue::new(1 << 20, 2);
        assert!(q.push(pkt(1, 0), Duration::from_millis(10)));
        assert!(q.push(pkt(2, 0), Duration::from_millis(10)));
        // no room: the push must time out, not block forever
        let t = Instant::now();
        assert!(!q.push(pkt(3, 0), Duration::from_millis(40)));
        assert!(t.elapsed() >= Duration::from_millis(30));
        q.pop(Duration::from_millis(10));
        assert!(q.push(pkt(3, 0), Duration::from_millis(50)));
    }

    #[test]
    fn respects_the_byte_cap() {
        let q = PacketQueue::new(25, 100);
        assert!(q.push(pkt(1, 0), Duration::from_millis(10)));
        assert!(q.push(pkt(2, 0), Duration::from_millis(10)));
        assert!(!q.push(pkt(3, 0), Duration::from_millis(30)));
        assert_eq!(q.bytes(), 20);
    }

    #[test]
    fn flush_and_abort() {
        let q = PacketQueue::new(1024, 10);
        q.push(pkt(1, 0), Duration::from_millis(10));
        q.flush();
        assert_eq!(q.len(), 0);
        assert_eq!(q.bytes(), 0);
        assert!(q.pop(Duration::from_millis(10)).is_none());
        q.abort();
        assert!(q.is_aborted());
        assert!(!q.push(pkt(2, 0), Duration::from_millis(10)));
        q.reset();
        assert!(!q.is_aborted());
        assert!(q.push(pkt(3, 1), Duration::from_millis(10)));
    }

    #[test]
    fn drop_stale_keeps_the_current_epoch() {
        let q = PacketQueue::new(1024, 10);
        q.push(pkt(1, 0), Duration::from_millis(10));
        q.push(pkt(2, 0), Duration::from_millis(10));
        q.push(pkt(3, 1), Duration::from_millis(10));
        assert_eq!(q.drop_stale(1), 2);
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop(Duration::from_millis(10)).unwrap().epoch, 1);
        assert_eq!(q.dropped(), 2);
    }

    #[test]
    fn buffered_span() {
        let q = PacketQueue::new(1 << 20, 10);
        assert_eq!(q.buffered_ms(), 0);
        q.push(pkt(1, 0), Duration::from_millis(10));
        q.push(pkt(5, 0), Duration::from_millis(10));
        assert_eq!(q.buffered_ms(), 4);
    }

    #[test]
    fn concurrent_push_pop() {
        use std::sync::Arc;
        use std::thread;
        let q = Arc::new(PacketQueue::new(1 << 16, 8));
        let producer = {
            let q = Arc::clone(&q);
            thread::spawn(move || {
                for i in 0..200u8 {
                    let mut ok = q.push(pkt(i, 0), Duration::from_millis(200));
                    while !ok {
                        ok = q.push(pkt(i, 0), Duration::from_millis(200));
                    }
                }
            })
        };
        let consumer = {
            let q = Arc::clone(&q);
            thread::spawn(move || {
                let mut n = 0;
                while n < 200 {
                    if q.pop(Duration::from_millis(200)).is_some() {
                        n += 1;
                    }
                }
                n
            })
        };
        producer.join().unwrap();
        assert_eq!(consumer.join().unwrap(), 200);
    }
}
