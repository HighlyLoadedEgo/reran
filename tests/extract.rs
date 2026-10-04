//! M2 extraction grammars: dynamic output → compact honest digests.
//! Trust contract: exit code is first-class in EVERY digest; failure is never
//! phrased as success (rtk#4421 false-green class).
use reran::digest::finalize_digest;
use reran::extract::extract;

#[test]
fn pytest_success_shows_counts_and_exit() {
    let raw = "tests/test_a.py ...\n============================= test session starts =============================\ntests/test_a.py ..\n\n============================== 3 passed in 0.12s ==============================\n";
    let d = extract(&["pytest".into(), "-q".into()], raw, 0);
    assert!(d.contains("3 passed"), "{d}");
    assert!(d.contains("pytest"), "{d}");
    assert!(d.contains("exit 0"), "{d}");
}

#[test]
fn pytest_failure_shows_failed_names_and_exit() {
    let raw = "FAILED tests/test_b.py::test_x - assert 1 == 2\n\n============================== 1 failed, 2 passed in 0.3s ==============================\n";
    let d = extract(&["pytest".into()], raw, 1);
    assert!(d.contains("1 failed"), "{d}");
    assert!(d.contains("2 passed"), "{d}");
    assert!(d.contains("test_b.py::test_x"), "{d}");
    assert!(d.contains("exit 1"), "{d}");
}

#[test]
fn pytest_collection_error_is_never_green() {
    // rtk#2317 class: ImportError at collection must not read as "no tests"/green
    let raw = "ImportError while importing test module 'tests/test_x.py'\nNo tests collected\n";
    let d = extract(&["pytest".into()], raw, 2);
    assert!(d.contains("exit 2"), "{d}");
    assert!(d.contains("error") || d.contains("FAILED") || d.contains("Import"), "{d}");
    assert!(!d.contains("passed"), "{d}");
}

#[test]
fn cargo_test_summary_and_failing_names() {
    let raw = "test parse::ok ... ok\ntest parse::bad ... FAILED\n\nfailures:\n    parse::bad\n\ntest result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n";
    let d = extract(&["cargo".into(), "test".into()], raw, 101);
    assert!(d.contains("2 passed"), "{d}");
    assert!(d.contains("1 failed"), "{d}");
    assert!(d.contains("parse::bad"), "{d}");
    assert!(d.contains("exit 101"), "{d}");
}

#[test]
fn go_test_failures() {
    let raw = "--- FAIL: TestSignup (0.00s)\n    user_test.go:42: no token\nFAIL\nFAIL example.com/pkg 0.5s\nok  \texample.com/other\t0.2s\n";
    let d = extract(&["go".into(), "test".into(), "./...".into()], raw, 1);
    assert!(d.contains("TestSignup"), "{d}");
    assert!(d.contains("1 failed"), "{d}");
    assert!(d.contains("1 ok"), "{d}");
    assert!(d.contains("exit 1"), "{d}");
}

#[test]
fn vitest_summary() {
    let raw = "Tests  3 failed | 15 passed (18)\n Test Files  2 failed | 4 passed (6)\n";
    let d = extract(&["vitest".into(), "run".into()], raw, 1);
    assert!(d.contains("15 passed"), "{d}");
    assert!(d.contains("3 failed"), "{d}");
    assert!(d.contains("exit 1"), "{d}");
}

#[test]
fn tsc_error_count_and_first_errors() {
    let raw = "src/a.ts(3,7): error TS2322: Type 'x' is not assignable to type 'y'.\nsrc/b.ts(9,1): error TS2304: Cannot find name 'z'.\nFound 2 errors in 2 files.\n";
    let d = extract(&["tsc".into(), "--noEmit".into()], raw, 2);
    assert!(d.contains("2 errors"), "{d}");
    assert!(d.contains("TS2322"), "{d}");
    assert!(d.contains("exit 2"), "{d}");
}

#[test]
fn curl_json_shows_top_level_keys() {
    let raw = r#"{"name":"reran","stars":82,"langs":["rust"],"meta":{"nested":true}}"#;
    let d = extract(&["curl".into(), "-s".into(), "https://api.example.com".into()], raw, 0);
    assert!(d.contains("JSON object"), "{d}");
    assert!(d.contains("name"), "{d}");
    assert!(d.contains("exit 0"), "{d}");
}

#[test]
fn unknown_output_gets_elision_digest() {
    let raw: String = (1..=50).map(|i| format!("line{i}\n")).collect();
    let d = extract(&["weird-tool".into(), "--run".into()], &raw, 0);
    assert!(d.contains("[+10 lines elided by reran]"), "{d}");
    assert!(d.contains("exit 0"), "{d}");
}

#[test]
fn finalize_wraps_with_unchanged_line_and_keeps_trust() {
    let inner = extract(&["pytest".into()], "1 failed, 1 passed in 0.1s\nFAILED t::x", 1);
    let full = finalize_digest(3, &inner);
    assert!(full.contains("unchanged since turn 3"), "{full}");
    assert!(full.contains("exit 1"), "{full}");
}

#[test]
fn digest_size_is_bounded() {
    let raw: String = "spam\n".repeat(5_000);
    let d = extract(&["cat".into(), "spam.log".into()], &raw, 0);
    assert!(d.len() < 2_000, "digest len {}", d.len());
}
