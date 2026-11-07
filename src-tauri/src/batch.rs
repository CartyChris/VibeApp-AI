//! Streamed text, batched before it crosses into the webview.
//!
//! Providers send a token or two per chunk. Each `emit` is a JSON encode, a
//! script evaluation in the page, and then a layout there, so a long reply
//! crossed the bridge thousands of times. About forty batches a second look
//! the same on screen and cost a small fraction of that.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Longest a chunk waits before it is shown.
pub const WINDOW: Duration = Duration::from_millis(25);
/// A burst this large goes out at once rather than waiting for the window.
const MAX_BYTES: usize = 32 * 1024;

type Sink = Arc<dyn Fn(&str, String) + Send + Sync>;

#[derive(Default)]
struct Pending {
    kind: String,
    text: String,
    since: Option<Instant>,
    closed: bool,
}

/// Cloning shares the same buffer.
#[derive(Clone)]
pub struct Deltas {
    sink: Sink,
    buf: Arc<Mutex<Pending>>,
}

/// Kinds whose chunks can be joined. Anything else (a status line, an image,
/// a file that was just applied) is an event in its own right.
fn joinable(kind: &str) -> bool {
    matches!(kind, "text" | "reasoning" | "plan")
}

impl Deltas {
    pub fn new(sink: impl Fn(&str, String) + Send + Sync + 'static) -> Self {
        Deltas { sink: Arc::new(sink), buf: Arc::new(Mutex::new(Pending::default())) }
    }

    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.buf.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The sink runs with the lock held, so events leave in the order they came.
    fn flush_locked(&self, p: &mut Pending) {
        if p.text.is_empty() {
            return;
        }
        let text = std::mem::take(&mut p.text);
        p.since = None;
        (self.sink)(&p.kind, text);
    }

    pub fn send(&self, kind: &str, text: String) {
        let mut p = self.lock();
        if p.closed {
            drop(p);
            (self.sink)(kind, text);
            return;
        }
        if !p.text.is_empty() && (!joinable(kind) || p.kind != kind) {
            self.flush_locked(&mut p);
        }
        if !joinable(kind) {
            (self.sink)(kind, text);
            return;
        }
        if p.text.is_empty() {
            p.kind = kind.to_string();
            p.since = Some(Instant::now());
        }
        p.text.push_str(&text);
        if p.text.len() >= MAX_BYTES || p.since.is_some_and(|s| s.elapsed() >= WINDOW) {
            self.flush_locked(&mut p);
        }
    }

    /// Called by the ticker every `WINDOW`. False once there is nothing left
    /// to tick for: the stream finished, or every other handle was dropped
    /// (an early return), in which case what was waiting goes out first.
    pub fn tick(&self) -> bool {
        let mut p = self.lock();
        if p.closed {
            return false;
        }
        self.flush_locked(&mut p);
        if Arc::strong_count(&self.buf) <= 1 {
            p.closed = true;
            return false;
        }
        true
    }

    /// Send what is waiting and stop batching. Call before the stream's final
    /// event, so nothing arrives after it.
    pub fn finish(&self) {
        let mut p = self.lock();
        self.flush_locked(&mut p);
        p.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder() -> (Deltas, Arc<Mutex<Vec<(String, String)>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        (Deltas::new(move |k, t| l.lock().unwrap().push((k.to_string(), t))), log)
    }

    fn got(log: &Arc<Mutex<Vec<(String, String)>>>) -> Vec<(String, String)> {
        log.lock().unwrap().clone()
    }

    #[test]
    fn a_burst_of_tokens_crosses_as_one_event() {
        let (d, log) = recorder();
        for t in ["Hel", "lo ", "wor", "ld"] {
            d.send("text", t.into());
        }
        assert!(got(&log).is_empty(), "nothing goes out inside the window");
        d.finish();
        assert_eq!(got(&log), vec![("text".into(), "Hello world".into())]);
    }

    #[test]
    fn order_is_kept_across_kinds() {
        let (d, log) = recorder();
        d.send("text", "a".into());
        d.send("text", "b".into());
        d.send("status", "Applied index.html".into());
        d.send("text", "c".into());
        d.send("reasoning", "r".into());
        d.send("text", "d".into());
        d.finish();
        let kinds: Vec<_> = got(&log).into_iter().map(|(k, t)| format!("{k}:{t}")).collect();
        assert_eq!(kinds, ["text:ab", "status:Applied index.html", "text:c", "reasoning:r", "text:d"]);
    }

    #[test]
    fn text_that_has_waited_a_window_goes_out_with_the_next_chunk() {
        let (d, log) = recorder();
        d.send("text", "a".into());
        std::thread::sleep(WINDOW + Duration::from_millis(5));
        d.send("text", "b".into());
        assert_eq!(got(&log), vec![("text".into(), "ab".into())]);
    }

    #[test]
    fn a_large_burst_does_not_wait() {
        let (d, log) = recorder();
        d.send("text", "x".repeat(MAX_BYTES));
        assert_eq!(got(&log).len(), 1);
    }

    #[test]
    fn the_ticker_flushes_a_stalled_stream_and_stops_when_abandoned() {
        let (d, log) = recorder();
        let ticker = d.clone();
        d.send("text", "last words before a long pause".into());
        assert!(ticker.tick(), "the stream is still open");
        assert_eq!(got(&log).len(), 1, "a pause must not hide what already arrived");
        d.send("text", "more".into());
        drop(d); // an early return that never called finish
        assert!(!ticker.tick());
        assert_eq!(got(&log).last().unwrap().1, "more");
        assert!(!ticker.tick());
    }

    #[test]
    fn after_finish_events_pass_straight_through() {
        let (d, log) = recorder();
        d.finish();
        d.send("text", "late".into());
        assert_eq!(got(&log), vec![("text".into(), "late".into())]);
        assert!(!d.tick());
    }
}
