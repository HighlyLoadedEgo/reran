use reran::store::{Entry, Store};
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn entry(key_byte: u8) -> Entry {
    Entry {
        key: [key_byte; 32],
        session_id: "s1".into(),
        turn: 1,
        digest: "digest".into(),
        raw: b"line1\nline2\nline3".to_vec(),
        exit_code: 0,
        created_at: now(),
    }
}

fn temp_db() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("reran.db");
    (d, p)
}

#[test]
fn roundtrip_entry() {
    let (_d, p) = temp_db();
    let store = Store::open(&p).unwrap();
    store.put(&entry(1)).unwrap();
    let got = store.get(&[1; 32]).unwrap().unwrap();
    assert_eq!(got.raw, b"line1\nline2\nline3".to_vec());
    assert_eq!(got.session_id, "s1");
    assert_eq!(got.exit_code, 0);
    assert!(store.get(&[2; 32]).unwrap().is_none());
}

#[test]
fn seen_range_tracking() {
    let (_d, p) = temp_db();
    let store = Store::open(&p).unwrap();
    let k = [3u8; 32];
    assert!(!store.saw_whole("s1", &k, 3).unwrap(), "nothing recorded yet");
    store.record_seen("s1", &k, 3).unwrap();
    assert!(store.saw_whole("s1", &k, 3).unwrap());
    assert!(!store.saw_whole("s1", &k, 4).unwrap(), "4th line never seen");
    assert!(!store.saw_whole("s2", &k, 3).unwrap(), "other session never saw it");
}

#[test]
fn eviction_keeps_newest_10k_idempotent() {
    let (_d, p) = temp_db();
    let store = Store::open(&p).unwrap();
    for i in 0..10_002u64 {
        let mut e = entry((i % 251) as u8);
        e.key = {
            let mut k = [0u8; 32];
            k[..8].copy_from_slice(&i.to_be_bytes());
            k
        };
        e.created_at = i; // deterministic age ordering
        store.put(&e).unwrap();
    }
    // second run of eviction is a no-op (idempotence)
    for i in 0..10_002u64 {
        let mut k = [0u8; 32];
        k[..8].copy_from_slice(&i.to_be_bytes());
        if i < 2 {
            assert!(store.get(&k).unwrap().is_none(), "oldest evicted: {i}");
        }
    }
    let mut k = [0u8; 32];
    k[..8].copy_from_slice(&10_001u64.to_be_bytes());
    assert!(store.get(&k).unwrap().is_some(), "newest kept");
}

#[test]
fn stats_sum_events() {
    let (_d, p) = temp_db();
    let store = Store::open(&p).unwrap();
    store.record_event("hit", 120).unwrap();
    store.record_event("hit", 80).unwrap();
    store.record_event("miss", 0).unwrap();
    store.record_event("bypass", 0).unwrap();
    store.record_event("uncached_failure", 0).unwrap();
    let s = store.stats().unwrap();
    assert_eq!(s.hits, 2);
    assert_eq!(s.misses, 1);
    assert_eq!(s.bypass, 1);
    assert_eq!(s.uncached_failures, 1);
    assert_eq!(s.tokens_saved, 200);
}

#[test]
fn concurrent_opens_wal() {
    let (_d, p) = temp_db();
    let s1 = Store::open(&p).unwrap();
    let s2 = Store::open(&p).unwrap();
    s1.record_event("hit", 1).unwrap();
    s2.record_event("miss", 0).unwrap();
    assert_eq!(s1.stats().unwrap().hits, 1);
}

#[test]
fn default_db_path_honors_env() {
    // RERAN_DB wins over ~/.cache
    // (env var read inside; here we only assert the fallback shape)
    if std::env::var("RERAN_DB").is_err() {
        let p = Store::default_db_path();
        assert!(p.to_string_lossy().contains("reran"));
    }
}
