use std::{collections::HashSet, path::Path, path::PathBuf, time::Duration};

use notify::{RecursiveMode, Watcher};

pub struct Watch {
    _watcher: notify::RecommendedWatcher,
    events: smol::channel::Receiver<PathBuf>,
}

impl Watch {
    pub fn new(root: &Path) -> notify::Result<Self> {
        let (tx, events) = smol::channel::unbounded();
        let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                for path in event.paths {
                    tx.try_send(path).ok();
                }
            }
        })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            events,
        })
    }

    // A checkout or build touches many files at once; one batch means one update.
    pub async fn next(&self) -> Option<HashSet<PathBuf>> {
        let first = self.events.recv().await.ok()?;
        smol::Timer::after(Duration::from_millis(100)).await;
        Some(std::iter::once(first).chain(std::iter::from_fn(|| self.events.try_recv().ok())).collect())
    }
}
