//! Downloaded pictures kept on disk, so a picture seen once is not fetched
//! again after a restart. Files are named by a hash of their URL and the
//! cache is capped: past the cap the least recently used files go (a read
//! touches the file's modification time). Nothing else is ever written.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

pub struct DiskCache {
    dir: PathBuf,
    cap: u64,
    /// Bytes on disk, kept in step with writes and evictions.
    total: Mutex<u64>,
}

impl std::fmt::Debug for DiskCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DiskCache({}, cap {} bytes)",
            self.dir.display(),
            self.cap
        )
    }
}

impl DiskCache {
    /// Open (creating) the cache directory. None when the cap is zero or
    /// the directory cannot be used; the client then just downloads.
    pub fn open(dir: PathBuf, cap: u64) -> Option<Self> {
        if cap == 0 {
            return None;
        }
        fs::create_dir_all(&dir).ok()?;
        let cache = Self {
            dir,
            cap,
            total: Mutex::new(0),
        };
        let total = cache.scan().map(|(t, _)| t).unwrap_or(0);
        *cache.total.lock().unwrap() = total;
        cache.evict_if_needed();
        Some(cache)
    }

    #[cfg(test)]
    pub fn bytes_used(&self) -> u64 {
        *self.total.lock().unwrap()
    }

    fn file_for(&self, url: &str) -> PathBuf {
        self.dir.join(format!("{:016x}", fnv1a64(url.as_bytes())))
    }

    /// The cached bytes for a URL, if any; a hit counts as recent use.
    pub fn read(&self, url: &str) -> Option<Vec<u8>> {
        let path = self.file_for(url);
        let bytes = fs::read(&path).ok()?;
        if bytes.is_empty() {
            return None;
        }
        if let Ok(f) = fs::File::options().write(true).open(&path) {
            let _ = f.set_modified(SystemTime::now());
        }
        Some(bytes)
    }

    /// Keep the bytes for a URL. A file bigger than a quarter of the cap is
    /// not worth keeping: it would push out everything else.
    pub fn write(&self, url: &str, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() || bytes.len() as u64 > self.cap / 4 {
            return Ok(());
        }
        let path = self.file_for(url);
        let tmp = path.with_extension("part");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)?;
        *self.total.lock().unwrap() += bytes.len() as u64;
        self.evict_if_needed();
        Ok(())
    }

    /// Total size and (mtime, size, path) of every cached file.
    #[allow(clippy::type_complexity)]
    fn scan(&self) -> io::Result<(u64, Vec<(SystemTime, u64, PathBuf)>)> {
        let mut files = Vec::new();
        let mut total = 0;
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            total += meta.len();
            files.push((mtime, meta.len(), entry.path()));
        }
        Ok((total, files))
    }

    /// Past the cap, drop the least recently used files down to three
    /// quarters of it, so eviction does not run on every write.
    fn evict_if_needed(&self) {
        if *self.total.lock().unwrap() <= self.cap {
            return;
        }
        let Ok((mut total, mut files)) = self.scan() else {
            return;
        };
        files.sort_by_key(|(mtime, _, _)| *mtime);
        let target = self.cap / 4 * 3;
        for (_, size, path) in files {
            if total <= target {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
        *self.total.lock().unwrap() = total;
    }
}

/// FNV-1a, 64 bit: a stable file name for a URL across builds.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn fnv1a64_matches_the_reference_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }

    #[test]
    fn round_trips_and_ignores_empty_or_huge_files() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::open(dir.path().join("media"), 1000).unwrap();
        cache.write("https://x/a.webp", b"hello").unwrap();
        assert_eq!(
            cache.read("https://x/a.webp").as_deref(),
            Some(&b"hello"[..])
        );
        assert_eq!(cache.read("https://x/b.webp"), None);
        cache.write("https://x/empty", b"").unwrap();
        assert_eq!(cache.read("https://x/empty"), None);
        cache.write("https://x/huge", &[1u8; 300]).unwrap();
        assert_eq!(cache.read("https://x/huge"), None);
        assert_eq!(cache.bytes_used(), 5);
        assert!(DiskCache::open(dir.path().join("off"), 0).is_none());
    }

    #[test]
    fn evicts_the_least_recently_used_first() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::open(dir.path().to_path_buf(), 100).unwrap();
        cache.write("old", &[0u8; 20]).unwrap();
        cache.write("mid", &[0u8; 20]).unwrap();
        cache.write("new", &[0u8; 20]).unwrap();
        // make the ages unambiguous
        let base = SystemTime::now() - Duration::from_secs(300);
        for (name, age) in [("old", 0), ("mid", 100), ("new", 200)] {
            let f = fs::File::options()
                .write(true)
                .open(cache.file_for(name))
                .unwrap();
            f.set_modified(base + Duration::from_secs(age)).unwrap();
        }
        // reading "old" makes it the most recent
        assert!(cache.read("old").is_some());
        // 60 + 25 + 25 = 110 > 100: evict down to 75 -> drops "mid" (oldest now)
        cache.write("more", &[0u8; 25]).unwrap();
        cache.write("more2", &[0u8; 25]).unwrap();
        assert!(
            cache.read("mid").is_none(),
            "mid was the least recently used"
        );
        assert!(cache.read("old").is_some());
        assert!(cache.bytes_used() <= 100);
        // the reopened cache knows what is on disk
        let reopened = DiskCache::open(dir.path().to_path_buf(), 100).unwrap();
        assert_eq!(reopened.bytes_used(), cache.bytes_used());
    }
}
