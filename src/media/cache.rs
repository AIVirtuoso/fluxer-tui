//! Pictures ready to draw, kept in memory within a byte budget. Entries are
//! keyed by URL and cell size; the least recently drawn go first when the
//! budget is exceeded, and whatever is on screen this draw is never evicted.

use crate::app::PictureFrames;
use std::cell::Cell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct ReadyMedia {
    pub frames: PictureFrames,
    pub bytes: usize,
    /// Draw serial of the last time it was drawn.
    last_used: Cell<u64>,
}

pub enum MediaState {
    Loading { since: Instant },
    Failed { at: Instant },
    Ready(ReadyMedia),
}

pub enum Lookup<'a> {
    Ready(&'a PictureFrames),
    /// Loading, or failed too recently to try again.
    Pending,
    Missing,
}

pub struct MediaCache {
    entries: HashMap<String, MediaState>,
    used: usize,
    budget: usize,
    in_flight: usize,
}

impl std::fmt::Debug for MediaCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MediaCache({} entries, {} of {} bytes, {} in flight)",
            self.entries.len(),
            self.used,
            self.budget,
            self.in_flight
        )
    }
}

impl MediaCache {
    /// Downloads and decodes running at once; the rest wait for a redraw.
    pub const MAX_IN_FLIGHT: usize = 4;
    /// A failed download is tried again after this long.
    pub const RETRY_AFTER: Duration = Duration::from_secs(60);
    /// A download that never reported back is given up on after this long.
    pub const LOADING_TIMEOUT: Duration = Duration::from_secs(120);

    pub fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            used: 0,
            budget,
            in_flight: 0,
        }
    }

    #[cfg(test)]
    pub fn budget(&self) -> usize {
        self.budget
    }

    #[cfg(test)]
    pub fn bytes_used(&self) -> usize {
        self.used
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The pictures for a key; a hit counts as use in draw `draw`.
    pub fn lookup(&self, key: &str, draw: u64) -> Lookup<'_> {
        match self.entries.get(key) {
            Some(MediaState::Ready(r)) => {
                r.last_used.set(draw);
                Lookup::Ready(&r.frames)
            }
            Some(MediaState::Loading { since }) if since.elapsed() < Self::LOADING_TIMEOUT => {
                Lookup::Pending
            }
            Some(MediaState::Failed { at }) if at.elapsed() < Self::RETRY_AFTER => Lookup::Pending,
            _ => Lookup::Missing,
        }
    }

    pub fn can_start(&self) -> bool {
        self.in_flight < Self::MAX_IN_FLIGHT
    }

    /// Mark a key as loading; false when it is already loading or ready.
    pub fn start(&mut self, key: &str) -> bool {
        if !self.can_start() {
            return false;
        }
        match self.entries.get(key) {
            Some(MediaState::Ready(_)) => return false,
            Some(MediaState::Loading { since }) if since.elapsed() < Self::LOADING_TIMEOUT => {
                return false;
            }
            Some(MediaState::Failed { at }) if at.elapsed() < Self::RETRY_AFTER => return false,
            _ => {}
        }
        self.entries.insert(
            key.to_string(),
            MediaState::Loading {
                since: Instant::now(),
            },
        );
        self.in_flight += 1;
        true
    }

    /// Store what a download produced (None: it failed), then make room.
    pub fn finish(&mut self, key: String, frames: Option<PictureFrames>, bytes: usize, draw: u64) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.remove(&key);
        match frames {
            Some(frames) => {
                self.used += bytes;
                self.entries.insert(
                    key,
                    MediaState::Ready(ReadyMedia {
                        frames,
                        bytes,
                        last_used: Cell::new(draw),
                    }),
                );
            }
            None => {
                self.entries
                    .insert(key, MediaState::Failed { at: Instant::now() });
            }
        }
        self.evict(draw);
    }

    fn remove(&mut self, key: &str) {
        if let Some(MediaState::Ready(r)) = self.entries.remove(key) {
            self.used = self.used.saturating_sub(r.bytes);
        }
    }

    /// Drop the least recently drawn pictures until the budget fits; what
    /// was drawn in draw `draw` stays.
    fn evict(&mut self, draw: u64) {
        while self.used > self.budget {
            let victim = self
                .entries
                .iter()
                .filter_map(|(k, v)| match v {
                    MediaState::Ready(r) if r.last_used.get() < draw => {
                        Some((r.last_used.get(), k.clone()))
                    }
                    _ => None,
                })
                .min();
            match victim {
                Some((_, key)) => self.remove(&key),
                None => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Picture;
    use std::sync::Arc;

    fn frames(px: u32) -> (PictureFrames, usize) {
        let img = image::RgbaImage::new(px, px);
        let bytes = img.len();
        (
            PictureFrames::new(vec![Picture::Pixels(Arc::new(img))], vec![Duration::ZERO]),
            bytes,
        )
    }

    #[test]
    fn keeps_what_is_on_screen_and_evicts_the_rest_oldest_first() {
        let mut cache = MediaCache::new(3 * 4 * 4 * 4); // room for three 4x4 pictures
        for (k, draw) in [("a", 1), ("b", 2), ("c", 3)] {
            assert!(cache.start(k));
            let (f, b) = frames(4);
            cache.finish(k.to_string(), Some(f), b, draw);
        }
        assert_eq!(cache.len(), 3);
        // draw 10 uses "a" and "c"; then "d" arrives and one must go: "b"
        assert!(matches!(cache.lookup("a", 10), Lookup::Ready(_)));
        assert!(matches!(cache.lookup("c", 10), Lookup::Ready(_)));
        assert!(cache.start("d"));
        let (f, b) = frames(4);
        cache.finish("d".to_string(), Some(f), b, 10);
        assert!(matches!(cache.lookup("b", 11), Lookup::Missing));
        assert!(matches!(cache.lookup("a", 11), Lookup::Ready(_)));
        assert!(matches!(cache.lookup("d", 11), Lookup::Ready(_)));
        assert!(cache.bytes_used() <= cache.budget());
    }

    #[test]
    fn nothing_drawn_this_draw_is_evicted_even_over_budget() {
        let mut cache = MediaCache::new(10);
        assert!(cache.start("big"));
        let (f, b) = frames(8);
        cache.finish("big".to_string(), Some(f), b, 5);
        assert!(matches!(cache.lookup("big", 5), Lookup::Ready(_)));
        assert!(cache.bytes_used() > cache.budget());
    }

    #[test]
    fn failures_are_not_retried_at_once_and_loads_are_limited() {
        let mut cache = MediaCache::new(1 << 20);
        assert!(cache.start("x"));
        assert!(!cache.start("x"), "already loading");
        assert!(matches!(cache.lookup("x", 1), Lookup::Pending));
        cache.finish("x".to_string(), None, 0, 1);
        assert!(matches!(cache.lookup("x", 2), Lookup::Pending));
        assert!(!cache.start("x"));
        for k in ["a", "b", "c", "d"] {
            assert!(cache.start(k));
        }
        assert!(!cache.can_start());
        assert!(!cache.start("e"));
        cache.finish("a".to_string(), None, 0, 3);
        assert!(cache.can_start());
    }
}
