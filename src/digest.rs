use crate::store::Entry;

pub const HEAD_LINES: usize = 20;
pub const TAIL_LINES: usize = 20;

/// head + tail with an explicit, machine-countable elision marker (spec §5.3:
/// every elision is explicit). Short inputs pass through verbatim.
pub fn elide(raw: &str, head: usize, tail: usize) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let n = lines.len();
    if n <= head + tail {
        return raw.to_string();
    }
    let elided = n - head - tail;
    let mut out: Vec<String> = Vec::with_capacity(head + tail + 1);
    for l in &lines[..head] {
        out.push((*l).to_string());
    }
    out.push(format!("[+{elided} lines elided by reran]"));
    for l in &lines[n - tail..] {
        out.push((*l).to_string());
    }
    out.join("\n")
}

pub fn estimate_tokens(s: &str) -> u64 {
    s.len() as u64 / 4
}

pub fn line_count(raw: &[u8]) -> u64 {
    if raw.is_empty() {
        return 0;
    }
    let mut n = raw.iter().filter(|&&b| b == b'\n').count() as u64;
    if *raw.last().unwrap() != b'\n' {
        n += 1;
    }
    n
}

/// Wrap a stored digest with the hit line. Legacy entries (pre-M2, empty digest)
/// fall back to the lines/tokens form.
pub fn finalize_digest(turn: u64, inner: &str) -> String {
    if inner.is_empty() {
        return format!("reran: unchanged since turn {turn}");
    }
    format!("reran: unchanged since turn {turn} · {inner}")
}

/// Render a cache hit. NEVER called for failed commands (engine guarantees exit 0),
/// and even then the wording contains no success vocabulary (false-green tripwire).
pub fn render_hit(e: &Entry) -> String {
    if !e.digest.is_empty() {
        return finalize_digest(e.turn, &e.digest);
    }
    let lines = line_count(&e.raw);
    let tokens = estimate_tokens(&String::from_utf8_lossy(&e.raw));
    format!(
        "reran: unchanged since turn {} · {lines} lines, ~{tokens} tokens",
        e.turn
    )
}
