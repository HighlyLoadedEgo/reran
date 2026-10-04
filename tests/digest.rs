use reran::digest::{elide, estimate_tokens, render_hit, TAIL_LINES, HEAD_LINES};
use reran::store::Entry;

fn entry(exit_code: i32) -> Entry {
    Entry {
        key: [7; 32],
        session_id: "s1".into(),
        turn: 3,
        digest: String::new(),
        raw: "out".as_bytes().to_vec(),
        exit_code,
        created_at: 0,
    }
}

#[test]
fn elide_keeps_head_and_tail_with_explicit_marker() {
    let raw: String = (1..=50).map(|i| format!("line{i}\n")).collect();
    let out = elide(&raw, 20, 20);
    assert!(out.contains("[+10 lines elided by reran]"), "{out}");
    let kept = out.lines().count();
    assert_eq!(kept, 41, "20 head + 1 marker + 20 tail, got {kept}");
    assert!(out.starts_with("line1\n"));
    assert!(out.contains("line50"));
    assert!(!out.contains("line30"));
}

#[test]
fn elide_keeps_short_input_verbatim() {
    let raw = "a\nb\nc";
    assert_eq!(elide(raw, 20, 20), raw);
}

#[test]
fn render_hit_names_turn_and_size_never_success_wording() {
    let mut e = entry(0);
    e.raw = "one\ntwo\nthree".as_bytes().to_vec();
    let out = render_hit(&e);
    assert!(out.contains("unchanged since turn 3"), "{out}");
    assert!(out.contains("3 lines"));
    // tripwire: hit rendering never announces success words (false-green class, rtk#4421)
    assert!(!out.contains("passed"));
    assert!(!out.contains(" ok"));
    // and a failed entry passed in by accident still cannot produce success wording
    let failed = entry(128);
    let out = render_hit(&failed);
    assert!(!out.contains("passed"));
}

#[test]
fn token_estimate_is_bytes_over_four() {
    assert_eq!(estimate_tokens("abcdefgh"), 2);
    assert_eq!(estimate_tokens(""), 0);
}

#[test]
fn head_tail_consts_are_twenty() {
    assert_eq!(HEAD_LINES, 20);
    assert_eq!(TAIL_LINES, 20);
}
