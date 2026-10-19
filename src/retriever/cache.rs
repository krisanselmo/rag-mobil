//! On-disk cache of inventories, so PlaymoDB isn't queried again for a set
//! already fetched. One JSON file per set, valid for [`DEFAULT_MAX_AGE`].

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::error::{RagError, Result};
use crate::model::{SetInventory, SetNumber};
use crate::retriever::InventorySource;

/// Inventories rarely change: a month avoids hitting PlaymoDB while still
/// picking up the community's corrections.
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

/// Per-user cache directory: `$XDG_CACHE_HOME/rag-mobil`, `~/.cache/rag-mobil`
/// (Linux), `~/Library/Caches/rag-mobil` (macOS), `%LOCALAPPDATA%\rag-mobil`
/// (Windows), else the temp directory.
pub fn default_cache_dir() -> PathBuf {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let base = if let Some(xdg) = env("XDG_CACHE_HOME") {
        PathBuf::from(xdg)
    } else if cfg!(windows) {
        env("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from)
    } else if let Some(home) = env("HOME") {
        if cfg!(target_os = "macos") {
            Path::new(&home).join("Library/Caches")
        } else {
            Path::new(&home).join(".cache")
        }
    } else {
        std::env::temp_dir()
    };
    base.join("rag-mobil")
}

/// Wraps a source with the cache.
pub struct Cached<S> {
    inner: S,
    dir: PathBuf,
    max_age: Duration,
    refresh: bool,
}

impl<S> Cached<S> {
    pub fn new(inner: S, dir: impl Into<PathBuf>) -> Self {
        Self {
            inner,
            dir: dir.into(),
            max_age: DEFAULT_MAX_AGE,
            refresh: false,
        }
    }

    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// Ignores cached entries (they are still rewritten after the fetch).
    pub fn refresh(mut self, refresh: bool) -> Self {
        self.refresh = refresh;
        self
    }

    fn path(&self, set: &SetNumber) -> PathBuf {
        self.dir.join(format!("{set}.json"))
    }

    /// The cached inventory, if present, fresh and readable.
    pub fn cached(&self, set: &SetNumber) -> Option<SetInventory> {
        if self.refresh {
            return None;
        }
        let path = self.path(set);
        let age = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())?;
        if age > self.max_age {
            return None;
        }
        // An unreadable entry (older format, partial write...) is refetched.
        let json = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str::<SetInventory>(&json)
            .ok()
            .filter(|inventory| &inventory.set_number == set)
    }

    fn store(&self, inventory: &SetInventory) -> Result<()> {
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| RagError::Io { path, source }
        };
        std::fs::create_dir_all(&self.dir).map_err(io(&self.dir))?;
        let path = self.path(&inventory.set_number);
        // Write then rename: a crash never leaves a half-written entry.
        let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_string_pretty(inventory)?).map_err(io(&tmp))?;
        std::fs::rename(&tmp, &path).map_err(io(&path))
    }
}

impl<S: InventorySource + Sync> InventorySource for Cached<S> {
    async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
        if let Some(inventory) = self.cached(set) {
            return Ok(inventory);
        }
        let inventory = self.inner.fetch(set).await?;
        // A cache that can't be written (read-only disk...) must not stop
        // the story: the inventory was fetched.
        let _ = self.store(&inventory);
        Ok(inventory)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct Counting {
        calls: AtomicUsize,
    }

    impl InventorySource for Counting {
        async fn fetch(&self, set: &SetNumber) -> Result<SetInventory> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(SetInventory {
                set_number: set.clone(),
                name: format!("fetch #{n}"),
                theme: None,
                focus: None,
                release_date: None,
                parts: vec![],
                source_url: "test".into(),
            })
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rag-mobil-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn set(n: &str) -> SetNumber {
        SetNumber::parse(n).unwrap()
    }

    #[tokio::test]
    async fn second_fetch_comes_from_the_cache() {
        let dir = temp_dir("hit");
        let cache = Cached::new(Counting::default(), &dir);
        assert_eq!(cache.fetch(&set("3799")).await.unwrap().name, "fetch #1");
        assert_eq!(cache.fetch(&set("3799")).await.unwrap().name, "fetch #1");
        assert_eq!(cache.fetch(&set("3666")).await.unwrap().name, "fetch #2");
        assert_eq!(cache.inner.calls.load(Ordering::SeqCst), 2);
        assert!(dir.join("3799.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn refresh_and_expiry_fetch_again() {
        let dir = temp_dir("refresh");
        let cache = Cached::new(Counting::default(), &dir);
        cache.fetch(&set("3799")).await.unwrap();

        let refreshing = Cached::new(Counting::default(), &dir).refresh(true);
        assert_eq!(
            refreshing.fetch(&set("3799")).await.unwrap().name,
            "fetch #1"
        );
        assert_eq!(refreshing.inner.calls.load(Ordering::SeqCst), 1);

        let expired = Cached::new(Counting::default(), &dir).with_max_age(Duration::ZERO);
        expired.fetch(&set("3799")).await.unwrap();
        assert_eq!(expired.inner.calls.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn corrupted_entry_is_fetched_again() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("3799.json"), "{ pas du json").unwrap();
        let cache = Cached::new(Counting::default(), &dir);
        assert_eq!(cache.fetch(&set("3799")).await.unwrap().name, "fetch #1");
        assert!(cache.cached(&set("3799")).is_some(), "entry rewritten");
        let _ = std::fs::remove_dir_all(dir);
    }
}
