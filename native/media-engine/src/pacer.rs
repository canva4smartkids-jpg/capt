//! Constant-frame-rate pacing of captured frames into the encoder.
//!
//! Backends deliver frames at their own cadence (WGC only on change, X11 at the requested
//! rate with jitter). The pacer ticks on the active clock and emits exactly one frame per
//! tick: the newest frame if one arrived, otherwise a duplicate of the previous one. Frames
//! arriving faster than the tick rate are superseded. Emitted frames go through a bounded
//! queue to the ffmpeg writer; when the encoder falls behind, new content is coalesced into
//! the last queued entry (counted as dropped) so the frame *count* — and therefore the
//! timing — stays exact.

use crate::clock::ActiveClock;
use capture_core::{Rect, Size, VideoFrame};
use parking_lot::{Condvar, Mutex};
use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacerDecision {
    New,
    Duplicate,
}

/// Pure scheduling logic (unit tested).
#[derive(Debug, Clone)]
pub struct PacerCore {
    fps: u32,
    emitted: u64,
    last_seq: Option<u64>,
    max_frames: Option<u64>,
}

impl PacerCore {
    pub fn new(fps: u32, max_duration_sec: Option<f64>) -> Self {
        let fps = fps.max(1);
        Self { fps, emitted: 0, last_seq: None, max_frames: max_duration_sec.map(|d| (d * fps as f64).round().max(1.0) as u64) }
    }

    pub fn emitted(&self) -> u64 {
        self.emitted
    }

    /// Frames that should have been emitted by active time `t`. Frame k (pts k/fps) is emitted
    /// once t >= (k - 0.5)/fps (frame 0 immediately), so a recording of T active seconds
    /// contains exactly round(T * fps) frames and each frame shows content captured within
    /// half a frame of its presentation time.
    pub fn frames_due(&self, active_sec: f64) -> u64 {
        if active_sec < 0.0 {
            return 0;
        }
        let due = ((active_sec * self.fps as f64 + 0.5 + 1e-9).floor() as u64).max(1);
        self.max_frames.map_or(due, |m| due.min(m))
    }

    /// Total frames for a recording of `total_sec` active seconds (never fewer than emitted).
    pub fn final_target(&self, total_sec: f64) -> u64 {
        let want = (total_sec.max(0.0) * self.fps as f64).round().max(1.0) as u64;
        let want = self.max_frames.map_or(want, |m| want.min(m));
        want.max(self.emitted)
    }

    /// Active time at which the next frame is due.
    pub fn next_due_sec(&self) -> f64 {
        self.emitted as f64 / self.fps as f64
    }

    /// Records one emitted frame given the newest frame sequence number available.
    pub fn next(&mut self, latest_seq: u64) -> PacerDecision {
        self.emitted += 1;
        if self.last_seq == Some(latest_seq) {
            PacerDecision::Duplicate
        } else {
            self.last_seq = Some(latest_seq);
            PacerDecision::New
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueEntry<T> {
    pub item: T,
    pub repeats: u32,
}

struct QState<T> {
    q: VecDeque<QueueEntry<T>>,
    closed: bool,
}

/// Bounded queue of distinct frames, each with a repeat count.
pub struct FrameQueue<T> {
    state: Mutex<QState<T>>,
    cv: Condvar,
    capacity: usize,
}

impl<T> FrameQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self { state: Mutex::new(QState { q: VecDeque::new(), closed: false }), cv: Condvar::new(), capacity: capacity.max(1) }
    }

    /// Queues new content. Returns `true` if the queue was full and the content replaced the
    /// newest pending entry instead (one distinct frame dropped, frame count preserved).
    pub fn push_new(&self, item: T) -> bool {
        let mut st = self.state.lock();
        let dropped = if st.q.len() >= self.capacity {
            match st.q.back_mut() {
                Some(last) => {
                    last.item = item;
                    last.repeats += 1;
                    true
                }
                None => false,
            }
        } else {
            st.q.push_back(QueueEntry { item, repeats: 1 });
            false
        };
        drop(st);
        self.cv.notify_one();
        dropped
    }

    /// Repeats the previous frame (`item` must be that frame).
    pub fn push_dup(&self, item: T) {
        let mut st = self.state.lock();
        match st.q.back_mut() {
            Some(last) => last.repeats += 1,
            None => st.q.push_back(QueueEntry { item, repeats: 1 }),
        }
        drop(st);
        self.cv.notify_one();
    }

    /// Blocks until an entry is available; `None` once closed and drained.
    pub fn pop(&self) -> Option<QueueEntry<T>> {
        let mut st = self.state.lock();
        loop {
            if let Some(e) = st.q.pop_front() {
                return Some(e);
            }
            if st.closed {
                return None;
            }
            self.cv.wait(&mut st);
        }
    }

    pub fn close(&self) {
        self.state.lock().closed = true;
        self.cv.notify_all();
    }

    pub fn len(&self) -> usize {
        self.state.lock().q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Frames (including repeats) waiting to be written.
    pub fn pending_frames(&self) -> u64 {
        self.state.lock().q.iter().map(|e| e.repeats as u64).sum()
    }
}

/// Latest frame delivered by the backend, with a monotonically increasing sequence number.
#[derive(Default)]
pub struct LatestFrame {
    slot: Mutex<Option<(u64, VideoFrame)>>,
    cv: Condvar,
}

impl LatestFrame {
    pub fn put(&self, frame: VideoFrame) {
        let mut g = self.slot.lock();
        let seq = g.as_ref().map_or(0, |(s, _)| s + 1);
        *g = Some((seq, frame));
        drop(g);
        self.cv.notify_all();
    }

    pub fn get(&self) -> Option<(u64, VideoFrame)> {
        self.slot.lock().clone()
    }

    pub fn wait_first(&self, timeout: Duration, abort: &dyn Fn() -> bool) -> Option<VideoFrame> {
        let deadline = Instant::now() + timeout;
        let mut g = self.slot.lock();
        loop {
            if let Some((_, f)) = g.as_ref() {
                return Some(f.clone());
            }
            if abort() || Instant::now() >= deadline {
                return None;
            }
            self.cv.wait_for(&mut g, Duration::from_millis(20));
        }
    }
}

/// Signals the pacer to emit the remaining frames for `total_sec` and exit.
#[derive(Default)]
pub struct PacerControl {
    finish: Mutex<Option<f64>>,
    cv: Condvar,
}

impl PacerControl {
    pub fn finish(&self, total_sec: f64) {
        *self.finish.lock() = Some(total_sec);
        self.cv.notify_all();
    }
}

/// Pacer thread body. Returns the number of frames emitted.
pub fn run_pacer(
    mut core: PacerCore,
    clock: Arc<Mutex<ActiveClock>>,
    latest: Arc<LatestFrame>,
    queue: Arc<FrameQueue<VideoFrame>>,
    ctl: Arc<PacerControl>,
    dropped: Arc<AtomicU64>,
) -> u64 {
    loop {
        let finish = *ctl.finish.lock();
        let (active, paused) = {
            let c = clock.lock();
            (c.now(), c.is_paused())
        };
        let target = match finish {
            Some(total) => core.final_target(total),
            None if paused => core.emitted(),
            None => core.frames_due(active),
        };
        while core.emitted() < target {
            let Some((seq, frame)) = latest.get() else { break };
            match core.next(seq) {
                PacerDecision::New => {
                    if queue.push_new(frame) {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                PacerDecision::Duplicate => queue.push_dup(frame),
            }
        }
        if finish.is_some() {
            return core.emitted();
        }
        let wait = if paused { 0.01 } else { (core.next_due_sec() - active).clamp(0.001, 0.02) };
        let mut g = ctl.finish.lock();
        if g.is_none() {
            ctl.cv.wait_for(&mut g, Duration::from_secs_f64(wait));
        }
    }
}

/// Converts captured frames to the fixed encoder input size: crops regions (when the
/// backend delivered the whole display) and crops/pads frames after a source resize.
#[derive(Debug, Clone)]
pub struct FramePrep {
    pub input: Size,
    /// Region rect relative to the delivered frame, applied unless the frame already has
    /// `region_size` (backend cropped natively).
    pub region: Option<(Rect, Size)>,
}

pub enum Prepared {
    Direct(VideoFrame),
    Owned(Vec<u8>),
}

impl Prepared {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Prepared::Direct(f) => &f.data[..f.width as usize * 4 * f.height as usize],
            Prepared::Owned(v) => v,
        }
    }
}

impl FramePrep {
    pub fn prepare(&self, frame: &VideoFrame) -> Prepared {
        let cropped;
        let mut f = frame;
        if let Some((rect, size)) = self.region {
            if !(frame.width == size.width && frame.height == size.height) {
                match frame.crop(rect) {
                    Ok(c) => {
                        cropped = c;
                        f = &cropped;
                    }
                    Err(_) => return Prepared::Owned(black(self.input)),
                }
            }
        }
        if f.width == self.input.width && f.height == self.input.height {
            if f.stride == f.width * 4 {
                return Prepared::Direct(f.clone());
            }
            return Prepared::Owned(f.packed_bgra().into_owned());
        }
        Prepared::Owned(fit_top_left(f, self.input))
    }
}

fn black(size: Size) -> Vec<u8> {
    let mut v = vec![0u8; size.width as usize * size.height as usize * 4];
    for px in v.chunks_exact_mut(4) {
        px[3] = 255;
    }
    v
}

/// Copies `f` into a black canvas of `size`, anchored top-left (crop or pad).
pub fn fit_top_left(f: &VideoFrame, size: Size) -> Vec<u8> {
    let mut out = black(size);
    let w = f.width.min(size.width) as usize;
    let h = f.height.min(size.height) as usize;
    let dst_row = size.width as usize * 4;
    for y in 0..h {
        let src = y * f.stride as usize;
        out[y * dst_row..y * dst_row + w * 4].copy_from_slice(&f.data[src..src + w * 4]);
    }
    out
}

/// Writer thread body: writes each queued frame `repeats` times to ffmpeg's stdin.
/// Calls `on_error` once if writing fails (encoder died / disk full) and keeps draining.
pub fn run_writer<W: Write>(queue: Arc<FrameQueue<VideoFrame>>, mut out: W, prep: FramePrep, encoded: Arc<AtomicU64>, on_error: impl FnOnce(std::io::Error)) {
    let mut on_error = Some(on_error);
    let mut failed = false;
    while let Some(entry) = queue.pop() {
        if failed {
            continue;
        }
        let prepared = prep.prepare(&entry.item);
        let bytes = prepared.bytes();
        for _ in 0..entry.repeats {
            if let Err(e) = out.write_all(bytes) {
                failed = true;
                if let Some(cb) = on_error.take() {
                    cb(e);
                }
                break;
            }
            encoded.fetch_add(1, Ordering::Relaxed);
        }
    }
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_due_follows_active_time() {
        let p = PacerCore::new(30, None);
        assert_eq!(p.frames_due(-0.1), 0);
        assert_eq!(p.frames_due(0.0), 1);
        assert_eq!(p.frames_due(0.0166), 1);
        assert_eq!(p.frames_due(1.5 / 30.0), 2);
        assert_eq!(p.frames_due(5.0), 150);
        assert_eq!(p.final_target(5.0), 150);
        assert_eq!(p.final_target(5.01), 150);
        let capped = PacerCore::new(30, Some(2.0));
        assert_eq!(capped.frames_due(10.0), 60);
        assert_eq!(capped.final_target(10.0), 60);
    }

    #[test]
    fn duplicates_when_no_new_frame_and_supersedes_fast_frames() {
        let mut p = PacerCore::new(30, None);
        assert_eq!(p.next(0), PacerDecision::New);
        assert_eq!(p.next(0), PacerDecision::Duplicate); // backend delivered nothing new
        assert_eq!(p.next(0), PacerDecision::Duplicate);
        assert_eq!(p.next(5), PacerDecision::New); // frames 1..4 superseded
        assert_eq!(p.emitted(), 4);
    }

    /// Simulates a backend at 10 fps and one at 100 fps feeding a 30 fps pacer for 5 s,
    /// including a 1 s pause: the output frame count must equal active_time * fps.
    #[test]
    fn simulated_schedule_keeps_exact_frame_count() {
        for backend_fps in [10.0_f64, 100.0] {
            let mut p = PacerCore::new(30, None);
            let (mut new, mut dup) = (0, 0);
            let mut active = 0.0;
            let mut wall = 0.0;
            while wall < 6.0 {
                wall += 0.004; // pacer wakeups with jitter-free 4 ms granularity
                let paused = (2.0..3.0).contains(&wall);
                if !paused {
                    active += 0.004;
                }
                let seq = (wall * backend_fps).floor() as u64;
                let target = if paused { p.emitted() } else { p.frames_due(active) };
                while p.emitted() < target {
                    match p.next(seq) {
                        PacerDecision::New => new += 1,
                        PacerDecision::Duplicate => dup += 1,
                    }
                }
            }
            let total = p.final_target(active);
            assert_eq!(total, 150, "5 s active at 30 fps");
            if backend_fps < 30.0 {
                assert!(dup > new, "slow backend must be duplicated (new {new}, dup {dup})");
            } else {
                assert_eq!(dup, 0, "fast backend never needs duplicates");
            }
        }
    }

    #[test]
    fn queue_coalesces_when_full() {
        let q: FrameQueue<u32> = FrameQueue::new(2);
        assert!(!q.push_new(1));
        q.push_dup(1);
        assert!(!q.push_new(2));
        assert!(q.push_new(3), "third distinct frame is coalesced");
        assert_eq!(q.len(), 2);
        assert_eq!(q.pending_frames(), 4, "frame count preserved");
        let a = q.pop().unwrap();
        assert_eq!((a.item, a.repeats), (1, 2));
        let b = q.pop().unwrap();
        assert_eq!((b.item, b.repeats), (3, 2));
        q.push_dup(3);
        assert_eq!(q.pop().unwrap().item, 3);
        q.close();
        assert!(q.pop().is_none());
    }

    fn frame(w: u32, h: u32, val: u8) -> VideoFrame {
        VideoFrame::new_bgra(w, h, w * 4, vec![val; (w * h * 4) as usize]).unwrap()
    }

    #[test]
    fn prep_crops_regions_and_pads_resized_frames() {
        let full = frame(100, 50, 7);
        let prep = FramePrep { input: Size { width: 20, height: 10 }, region: Some((Rect { x: 10, y: 10, width: 20, height: 10 }, Size { width: 20, height: 10 })) };
        assert_eq!(prep.prepare(&full).bytes().len(), 20 * 10 * 4);
        // backend already cropped
        let native = frame(20, 10, 9);
        assert!(matches!(prep.prepare(&native), Prepared::Direct(_)));
        // resized source: smaller frame is padded with opaque black
        let prep2 = FramePrep { input: Size { width: 4, height: 4 }, region: None };
        let small = frame(2, 2, 200);
        let out = prep2.prepare(&small);
        let b = out.bytes();
        assert_eq!(b.len(), 64);
        assert_eq!(&b[0..4], &[200, 200, 200, 200]);
        assert_eq!(&b[8..12], &[0, 0, 0, 255]);
        // larger frame is cropped
        let big = frame(8, 8, 1);
        assert_eq!(prep2.prepare(&big).bytes().len(), 64);
    }

    #[test]
    fn writer_writes_repeats() {
        let q = Arc::new(FrameQueue::new(4));
        q.push_new(frame(2, 2, 1));
        q.push_dup(frame(2, 2, 1));
        q.push_new(frame(2, 2, 2));
        q.close();
        let encoded = Arc::new(AtomicU64::new(0));
        let mut sink = Vec::new();
        run_writer(q, &mut sink, FramePrep { input: Size { width: 2, height: 2 }, region: None }, encoded.clone(), |_| {});
        assert_eq!(encoded.load(Ordering::Relaxed), 3);
        assert_eq!(sink.len(), 3 * 16);
        assert_eq!(sink[32], 2);
    }

    #[test]
    fn pacer_thread_emits_for_active_time() {
        let clock = Arc::new(Mutex::new(ActiveClock::new()));
        let latest = Arc::new(LatestFrame::default());
        latest.put(frame(2, 2, 1));
        let queue = Arc::new(FrameQueue::new(1000));
        let ctl = Arc::new(PacerControl::default());
        let dropped = Arc::new(AtomicU64::new(0));
        clock.lock().start(Instant::now());
        let h = {
            let (c, l, q, k, d) = (clock.clone(), latest.clone(), queue.clone(), ctl.clone(), dropped.clone());
            std::thread::spawn(move || run_pacer(PacerCore::new(30, None), c, l, q, k, d))
        };
        std::thread::sleep(Duration::from_millis(300));
        clock.lock().pause(Instant::now());
        let at_pause = queue.pending_frames();
        std::thread::sleep(Duration::from_millis(300));
        assert!(queue.pending_frames() <= at_pause + 1, "no frames while paused");
        let total = clock.lock().now();
        ctl.finish(total);
        let emitted = h.join().unwrap();
        let expect = (total * 30.0).round() as u64;
        assert_eq!(emitted, expect.max(1));
        assert_eq!(queue.pending_frames(), emitted);
    }
}
