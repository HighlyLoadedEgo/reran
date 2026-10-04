use crate::classify::{classify, Class};
use crate::digest::{estimate_tokens, line_count, render_hit};
use crate::fsepoch::{marker_path, fs_epoch};
use crate::key::{build_key, env_allowlist_for, uid, CallCtx};
use crate::store::Store;
use std::fs;
use std::io::Write;

/// Outputs larger than this are never written into the store (F4).
pub const MAX_CACHED_OUTPUT_BYTES: usize = 262_144; // 256 KiB (F4)
use std::path::Path;

#[derive(Debug, Clone)]
pub enum Outcome {
    /// This session previously saw the output of this exact call.
    Hit { digest: String },
    /// Run for real; caller must call `record` afterwards.
    Miss,
    /// Write/unknown command — run raw, never cached.
    Bypass,
}

fn make_key(store: &Store, argv: &[String], cwd: &Path) -> ([u8; 32], CallCtx) {
    let ctx = CallCtx {
        cwd: cwd.to_path_buf(),
        argv: argv.to_vec(),
        uid: uid(),
        fs_epoch: fs_epoch(store.cache_dir(), cwd),
    };
    let env = env_allowlist_for(argv);
    (build_key(&ctx, &env), ctx)
}

pub fn evaluate(store: &Store, argv: &[String], cwd: &Path, session: &str) -> Outcome {
    if argv.is_empty() {
        return Outcome::Bypass;
    }
    if classify(argv) == Class::Bypass {
        let _ = store.record_event("bypass", 0);
        return Outcome::Bypass;
    }
    let (key, _ctx) = make_key(store, argv, cwd);
    let hit = store.get(&key).ok().flatten().filter(|e| {
        e.exit_code == 0 && store.saw_whole(session, &key, line_count(&e.raw)).unwrap_or(false)
    });
    match hit {
        Some(e) => {
            let digest = render_hit(&e);
            let saved = estimate_tokens(&String::from_utf8_lossy(&e.raw))
                .saturating_sub(estimate_tokens(&digest));
            let _ = store.record_event("hit", saved);
            Outcome::Hit { digest }
        }
        None => {
            let _ = store.record_event("miss", 0);
            Outcome::Miss
        }
    }
}

/// Monotonic per-session turn proxy: how many distinct commands this session has cached + 1.
fn next_turn(store: &Store, session: &str) -> u64 {
    store.session_turn(session).unwrap_or(0) + 1
}

pub fn record(store: &Store, argv: &[String], cwd: &Path, session: &str, raw: &str, exit_code: i32) {
    if argv.is_empty() {
        return;
    }
    if classify(argv) == Class::Bypass {
        // touch the cwd marker ⇒ every later evaluate sees a bumped epoch (spec §4.2)
        let marker = marker_path(store.cache_dir(), cwd);
        if let Some(parent) = marker.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(mut f) = fs::File::create(&marker) {
            let _ = f.write_all(b"write recorded");
        }
        return;
    }
    if raw.len() > MAX_CACHED_OUTPUT_BYTES {
        let _ = store.record_event("uncached_failure", 0);
        return;
    }
    if exit_code != 0 {
        let _ = store.record_event("uncached_failure", 0); // bash-cache#25: failures never cached
        return;
    }
    let (key, _ctx) = make_key(store, argv, cwd);
    let lock_path = marker_path(store.cache_dir(), cwd)
        .with_file_name(format!("{}.lock", hex(&key)));
    let _ = fs::create_dir_all(lock_path.parent().unwrap());
    let lock_file = match fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(_) => return, // fail-open: uncached, never fatal
    };
    use fs2::FileExt;
    if lock_file.lock_exclusive().is_err() {
        return;
    }
    let turn = next_turn(store, session);
    let e = crate::store::Entry {
        key,
        session_id: session.to_string(),
        turn,
        digest: String::new(),
        raw: raw.as_bytes().to_vec(),
        exit_code,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    let _ = store.put(&e);
    let _ = store.record_seen(session, &key, line_count(&e.raw));
    let _ = lock_file.unlock();
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
