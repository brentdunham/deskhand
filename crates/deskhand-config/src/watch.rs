//! Reloads the config file when it changes.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use crate::config::{Config, ConfigError};

/// Lets an editor finish writing before the file is read.
const SETTLE: Duration = Duration::from_millis(150);

/// Polls the file's modified time and size. Polling one small file is
/// negligible work, and unlike change notifications it behaves the same for
/// editors that save in place and editors that write a temp file and rename
/// it over the original.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

type Fingerprint = Option<(SystemTime, u64)>;

fn fingerprint(path: &PathBuf) -> Fingerprint {
    let m = fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

impl Watcher {
    /// Calls `on_change` with the reloaded config (or the error) each time
    /// the file changes after this call returns. A deleted file is ignored
    /// until it reappears.
    ///
    /// To avoid missing an edit made while starting up, spawn the watcher
    /// before the initial load.
    pub fn spawn<F>(path: PathBuf, interval: Duration, mut on_change: F) -> Self
    where
        F: FnMut(Result<Config, ConfigError>) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        // Taken here, not in the thread, so no change after `spawn` returns
        // can slip in before the baseline.
        let mut last = fingerprint(&path);
        let thread = thread::Builder::new()
            .name("deskhand-config-watch".into())
            .spawn(move || {
                loop {
                    thread::park_timeout(interval);
                    if thread_stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let now = fingerprint(&path);
                    if now == last {
                        continue;
                    }
                    if now.is_none() {
                        last = None;
                        continue;
                    }
                    thread::sleep(SETTLE);
                    last = fingerprint(&path);
                    on_change(Config::load(&path));
                }
            })
            .expect("failed to spawn config watcher thread");
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn reports_valid_and_invalid_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "").unwrap();

        let (tx, rx) = mpsc::channel();
        let watcher = Watcher::spawn(path.clone(), Duration::from_millis(20), move |r| {
            let _ = tx.send(r);
        });

        // Different length guarantees a new fingerprint even on filesystems
        // with coarse modification times.
        fs::write(&path, "hotkey = \"Ctrl+Alt+K\"").unwrap();
        let got = rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        assert_eq!(got.hotkey.to_string(), "Ctrl+Alt+K");

        fs::write(&path, "hotkey = \"K\"").unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().is_err());

        // Deletion is ignored.
        fs::remove_file(&path).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());

        drop(watcher);
    }

    #[test]
    fn drop_stops_promptly() {
        let dir = tempfile::tempdir().unwrap();
        let w = Watcher::spawn(dir.path().join("x.toml"), Duration::from_secs(60), |_| {});
        let start = std::time::Instant::now();
        drop(w);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
