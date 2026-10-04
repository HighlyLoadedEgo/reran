use sha2::Digest;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Heavy directories excluded from scans (Review Focus #4: scan must stay in ms).
pub const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", ".venv", "__pycache__"];

/// Hard cap on visited entries; exceeding it returns None (caller falls back to marker-only).
pub const VISIT_CAP: usize = 100_000;

pub fn latest_mtime(root: &Path, skip: &[&str], visit_cap: usize) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut worklist: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut visited: usize = 0;

    while let Some(dir) = worklist.pop() {
        let read = match fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in read.flatten() {
            visited += 1;
            if visited > visit_cap {
                return None; // budget exceeded — signal fallback, never a stale answer
            }
            let path = entry.path();
            let meta = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if skip.contains(&name) {
                    continue;
                }
                worklist.push(path);
            } else if meta.is_file() {
                let mtime = meta.modified().ok()?;
                if newest.is_none_or(|n| mtime > n) {
                    newest = Some(mtime);
                }
            }
        }
    }
    newest
}

pub fn marker_path(cache_dir: &Path, cwd: &Path) -> PathBuf {
    let mut hasher = sha2::Sha256::new();
    hasher.update(cwd.to_string_lossy().as_bytes());
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    cache_dir.join("markers").join(format!("{hex}.marker"))
}

fn mtime_as_u64(t: Option<SystemTime>) -> u64 {
    t.map(|t| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    })
    .unwrap_or(0)
}

pub fn fs_epoch(cache_dir: &Path, cwd: &Path) -> u64 {
    let scanned = latest_mtime(cwd, SKIP_DIRS, VISIT_CAP);
    let marker = fs::metadata(marker_path(cache_dir, cwd))
        .and_then(|m| m.modified())
        .ok();
    // scan cap hit (None) ⇒ marker-only epoch, still correct: every reran-recorded
    // write bumps the marker, so marker-only stays conservative for agent-caused changes.
    std::cmp::max(mtime_as_u64(scanned), mtime_as_u64(marker))
}
