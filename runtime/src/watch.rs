use anyhow::{Result, ensure};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
struct Observation {
    generation: u64,
    uncertain: bool,
}
struct Watch {
    state: Arc<Mutex<Observation>>,
    _watcher: RecommendedWatcher,
    started: Instant,
}
#[derive(Default)]
pub struct LiveSources {
    roots: BTreeMap<PathBuf, Watch>,
    cache: VecDeque<(String, Value, usize)>,
    bytes: usize,
    pub hits: u64,
    pub misses: u64,
}
impl LiveSources {
    fn register(root: &Path) -> Result<Watch> {
        let identity = same_file::Handle::from_path(root)?;
        let state = Arc::new(Mutex::new(Observation {
            generation: 1,
            uncertain: false,
        }));
        let observed = state.clone();
        let watched = root.to_owned();
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            if std::env::var_os("FLUXR_WATCH_TRACE").is_some() {
                eprintln!("observed {watched:?}: {result:?}");
            }
            let mut o = observed.lock().unwrap();
            match result {
                Ok(e)
                    if e.need_rescan()
                        || e.paths.is_empty()
                        || e.paths.iter().any(|p| p == &watched) =>
                {
                    o.uncertain = true;
                    o.generation += 1
                }
                Ok(e) if e.paths.iter().any(|p| p.starts_with(&watched)) => o.generation += 1,
                Err(_) => {
                    o.uncertain = true;
                    o.generation += 1
                }
                _ => (),
            }
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        if let Some(parent) = root.parent() {
            watcher.watch(parent, RecursiveMode::NonRecursive)?
        }
        ensure!(
            identity == same_file::Handle::from_path(root)?,
            "Root replaced during watch registration"
        );
        Ok(Watch {
            state,
            _watcher: watcher,
            started: Instant::now(),
        })
    }
    pub fn observe(&mut self, root: &Path) -> Result<Value> {
        let renew = self.roots.get(root).is_none_or(|w| {
            w.state.lock().unwrap().uncertain || w.started.elapsed() > Duration::from_secs(30)
        });
        if renew {
            let previous = self
                .roots
                .remove(root)
                .map(|w| w.state.lock().unwrap().generation + 1)
                .unwrap_or(1);
            self.cache.clear();
            self.bytes = 0;
            if self.roots.len() >= 128 {
                return Ok(Value::Null);
            }
            let Ok(w) = Self::register(root) else {
                return Ok(Value::Null);
            };
            w.state.lock().unwrap().generation += previous;
            self.roots.insert(root.into(), w);
        }
        let observation = self.roots[root].state.lock().unwrap();
        if observation.uncertain {
            return Ok(Value::Null);
        }
        Ok(json!(observation.generation))
    }
    pub fn get(&mut self, key: &str) -> Value {
        if let Some(idx) = self.cache.iter().position(|(k, _, _)| k == key) {
            let entry = self.cache.remove(idx).unwrap();
            let value = entry.1.clone();
            self.cache.push_back(entry);
            self.hits += 1;
            value
        } else {
            self.misses += 1;
            Value::Null
        }
    }
    pub fn put(&mut self, key: String, value: Value) -> Result<()> {
        let size = key.len() + serde_json::to_vec(&value)?.len();
        ensure!(key.len() <= 8192, "Cache key exceeds bound");
        if size > 2 * 1024 * 1024 {
            return Ok(());
        }
        if let Some(idx) = self.cache.iter().position(|(k, _, _)| *k == key) {
            self.bytes -= self.cache.remove(idx).unwrap().2
        }
        while self.bytes + size > 16 * 1024 * 1024 || self.cache.len() >= 256 {
            if let Some((_, _, n)) = self.cache.pop_front() {
                self.bytes -= n
            } else {
                break;
            }
        }
        self.bytes += size;
        self.cache.push_back((key, value, size));
        Ok(())
    }
}
