use reran::fsepoch::{fs_epoch, latest_mtime, marker_path, SKIP_DIRS, VISIT_CAP};
use std::fs;
use std::time::SystemTime;
use tempfile::tempdir;

#[test]
fn sees_newest_file_and_skips_heavy_dirs() {
    let d = tempdir().unwrap();
    fs::create_dir_all(d.path().join(".git")).unwrap();
    fs::create_dir_all(d.path().join("node_modules/pkg")).unwrap();
    fs::write(d.path().join(".git/HEAD"), "x").unwrap();
    fs::write(d.path().join("node_modules/pkg/i.js"), "x").unwrap();
    let f1 = fs::File::options()
        .write(true)
        .open(d.path().join(".git/HEAD"))
        .unwrap();
    f1.set_modified(SystemTime::UNIX_EPOCH).unwrap();
    let f2 = fs::File::options()
        .write(true)
        .open(d.path().join("node_modules/pkg/i.js"))
        .unwrap();
    f2.set_modified(SystemTime::UNIX_EPOCH).unwrap();
    fs::write(d.path().join("src.txt"), "new").unwrap();
    let m = latest_mtime(d.path(), SKIP_DIRS, VISIT_CAP).unwrap();
    assert!(
        m.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() > 1_600_000_000,
        "skipped dirs must not win: got {m:?}"
    );
}

#[test]
fn empty_dir_returns_none() {
    let d = tempdir().unwrap();
    assert_eq!(latest_mtime(d.path(), SKIP_DIRS, VISIT_CAP), None);
}

#[test]
fn epoch_is_max_of_scan_and_marker() {
    let cache = tempdir().unwrap();
    let cwd = tempdir().unwrap();
    let base = fs_epoch(cache.path(), cwd.path());
    assert_eq!(base, 0, "nothing scanned, no marker");

    fs::write(cwd.path().join("a.txt"), "x").unwrap();
    let with_file = fs_epoch(cache.path(), cwd.path());
    assert!(with_file > 0);

    // touch marker into the (distant) future → epoch must move forward
    let mp = marker_path(cache.path(), cwd.path());
    fs::create_dir_all(mp.parent().unwrap()).unwrap();
    fs::write(&mp, "x").unwrap();
    let new = fs::File::options().write(true).open(&mp).unwrap();
    new.set_modified(SystemTime::now() + std::time::Duration::from_secs(3600))
        .unwrap();
    let after_marker = fs_epoch(cache.path(), cwd.path());
    assert!(after_marker > with_file, "marker must be able to bump epoch");
}

#[test]
fn visit_cap_returns_none() {
    let d = tempdir().unwrap();
    fs::write(d.path().join("a"), "x").unwrap();
    assert_eq!(latest_mtime(d.path(), &[], 0), None, "cap 0 ⇒ None");
}
