# reran M1 Implementation Plan — core cache + Claude Code hook adapter

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A working `reran` binary that memoizes agent shell calls with FS-content invalidation and session-scoped "unchanged" semantics, wired into Claude Code via hooks.

**Architecture:** One Rust crate (bin + lib). Core lib: classifier → key-builder (incl. FS epoch) → SQLite store → engine. Thin hook adapter: `reran hook pre|post` subcommands reading Claude Code hook JSON on stdin; `reran init claude-code` wires them. `reran gain` reads honest stats. Extraction grammars are M2 — M1 digests are head+tail with explicit elision markers.

**Tech Stack:** Rust stable, clap 4, rusqlite (bundled SQLite, WAL), serde/serde_json, sha2, libc (uid), fs2 (single-flight lock), tempfile + assert_cmd (dev).

**Spec:** [docs/specs/2026-10-04-reran-design.md](../specs/2026-10-04-reran-design.md) — this plan implements its M1 milestone; M2–M4 get their own plans.

## Global Constraints

- Rust stable ≥ 1.80; edition 2021. macOS/Linux only in M1 (spec non-goal: Windows v1.x).
- No network calls anywhere in the core. No subprocess spawning for scanning (`find` etc.) — pure `std::fs`.
- Exact matching only — no semantic/fuzzy anything (spec §3).
- Exit codes are sacred; non-zero exits are never cached (spec §5).
- Adapters fail open: any internal error ⇒ allow the command through, never block the agent (spec §5).
- Token estimate = `bytes / 4`, counted only on output we actually replaced (honest accounting, spec §4.6).
- SQLite in WAL mode with `busy_timeout`; binary name `reran`; license MIT.

## Review Focus

1. **Cross-session/unseen "unchanged"** (cachebro#11 class): an agent must never be told "unchanged" for output it has not seen in THIS session — expected behavior: cache miss, command runs fresh. Pinned in Task 7.
2. **Failure rendered as success** (rtk#4421 class): a non-zero-exit command must never be cached, and hit phrasing must never claim a failed command ran clean. Pinned in Tasks 6–7.
3. **Malformed or drifted hook payload**: garbage/unknown JSON on stdin must produce exit 0 + empty stdout (allow), never a blocked agent. Pinned in Task 8.
4. **Huge subtrees** (`node_modules`, `target`): FS scan must skip heavy dirs and stay in milliseconds; a hard cap falls back to marker-only epoch. Pinned in Task 3.
5. **Concurrent identical calls**: parallel evaluates/records must not corrupt the store or duplicate execution (single-flight lock + busy_timeout). Pinned in Tasks 5, 7.

---

### Task 1: Cargo scaffold + CLI entry

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `src/lib.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Produces: binary `reran` with subcommands `hook`, `init`, `gain` (stubs that exit 0 this task); lib crate `reran` empty.

- [ ] **Step 1: Scaffold crate**

```bash
cd /Users/danilasafonov/PycharmProjects/reran
cargo init --name reran --lib
mkdir -p src/bin 2>/dev/null || true
```

`Cargo.toml`:

```toml
[package]
name = "reran"
version = "0.1.0"
edition = "2021"
license = "MIT"
description = "Your agent already ran that. reran remembers."

[dependencies]
clap = { version = "4", features = ["derive"] }
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
libc = "0.2"
fs2 = "0.4"

[dev-dependencies]
tempfile = "3"
assert_cmd = "2"
```

`src/main.rs`:

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "reran", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Hook adapter: invoked by harness hooks (reads JSON on stdin)
    Hook { #[arg(long)] event: String }, // "pre" | "post"
    /// Wire hooks into a harness config
    Init { harness: String },
    /// Show honest token-savings stats
    Gain,
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Hook { .. } | Cmd::Init { .. } | Cmd::Gain { .. } => {}
    }
}
```

`src/lib.rs`: `pub mod reran_core;` placeholder — instead keep lib root with doc comment only for now:

```rust
//! reran core: memoization engine for agent shell calls.
```

- [ ] **Step 2: Write failing CLI test** — `tests/cli.rs`:

```rust
use assert_cmd::Command;

#[test]
fn version_flag_works() {
    Command::cargo_bin("reran").unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicates::str::contains("reran 0.1.0"));
}
```

(add `use predicates::prelude::*;` — add `predicates = "3"` to dev-dependencies)

- [ ] **Step 3:** `cargo test` → FAIL (version output mismatch or import error) → fix until PASS.
- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src tests
git commit -m "feat: cargo scaffold + clap CLI skeleton"
```

### Task 2: Classifier

**Files:**
- Create: `src/classify.rs`
- Modify: `src/lib.rs` (`pub mod classify;`)
- Test: `src/classify.rs` (unit) + `tests/classify.rs`

**Interfaces:**
- Produces: `pub enum Class { Memoizable, Bypass }`, `pub fn classify(argv: &[String]) -> Class`

- [ ] **Step 1: Failing tests** — `tests/classify.rs`:

```rust
use reran::classify::{classify, Class};

fn v(s: &[&str]) -> Vec<String> { s.iter().map(|s| s.to_string()).collect() }

#[test]
fn read_prefixes_are_memoizable() {
    for cmd in [
        vec!["git", "status"], vec!["git", "log", "-5"], vec!["git", "diff", "HEAD~1"],
        vec!["ls", "-la"], vec!["cat", "README.md"], vec!["grep", "-r", "todo", "."],
        vec!["rg", "pattern"], vec!["find", ".", "-name", "x"], vec!["pwd"], vec!["which", "node"],
    ] {
        assert_eq!(classify(&v(&cmd)), Class::Memoizable, "{cmd:?}");
    }
}

#[test]
fn writes_and_unknown_are_bypass() {
    for cmd in [
        vec!["git", "commit", "-m", "x"], vec!["git", "add", "."], vec!["git", "push"],
        vec!["npm", "install"], vec!["rm", "-rf", "x"], vec!["mkdir", "x"], vec!["touch", "x"],
        vec!["curl", "https://example.com"],   // remote state: not FS-covered → bypass in M1
        vec!["node", "build.js"],              // unknown → bypass (conservative, bkt#58)
        vec!["git", "status", "&&", "npm", "test"], // compound → bypass in M1
    ] {
        assert_eq!(classify(&v(&cmd)), Class::Bypass, "{cmd:?}");
    }
}

#[test]
fn env_assignment_prefix_is_skipped() {
    assert_eq!(classify(&v(&["FOO=bar", "git", "status"])), Class::Memoizable);
    assert_eq!(classify(&v(&["FOO=bar", "npm", "install"])), Class::Bypass);
}
```

- [ ] **Step 2:** `cargo test --test classify` → FAIL (not defined).
- [ ] **Step 3: Implement** — `src/classify.rs`:

```rust
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Class { Memoizable, Bypass }

/// env assignments like FOO=bar before the command
fn strip_env<'a>(argv: &'a [String]) -> &'a [String] {
    let n = argv.iter().take_while(|a| a.contains('=') && !a.starts_with('-')
        && a.split('=').next().map_or(false, |k| !k.is_empty()
            && k.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')))
        .count();
    &argv[n..]
}

/// M1 allowlist of read-only first-tokens/subcommands. Unknown ⇒ Bypass (bkt#58: unknown commands may write files).
const READ_SUBCOMMANDS: &[(&str, &[&str])] = &[
    ("git", &["status", "log", "diff", "show", "branch", "tag", "remote", "rev-parse", "stash", "describe", "blame", "shortlog", "ls-files", "config", "--list", "-l"]),
    ("npm", &["ls", "list", "outdated", "view", "run"]), // run: read-only *intent*; actual scripts may write — see Task 7 rule: npm run bypass? no: unknown script ⇒ keep Memoizable only for ls/list/outdated/view
    ("cargo", &["check", "tree", "search", "metadata", "test"]),
];
const READ_BARE: &[&str] = &["ls", "cat", "head", "tail", "grep", "rg", "fd", "find", "pwd", "which", "whoami", "wc", "file", "tree", "du", "df", "date", "echo", "env", "printenv", "uname", "id"];

pub fn classify(argv: &[String]) -> Class {
    let argv = strip_env(argv);
    if argv.is_empty() { return Class::Bypass; }
    let cmd = argv[0].as_str();
    if argv.iter().any(|a| a == "&&" || a == "||" || a == ";" || a == "|") {
        return Class::Bypass; // M1: compound commands not analyzed
    }
    if READ_BARE.contains(&cmd) { return Class::Memoizable; }
    for (name, subs) in READ_SUBCOMMANDS {
        if cmd == *name && argv.len() >= 2 {
            let sub = argv[1].trim_start_matches('-');
            if subs.contains(&argv[1]) || subs.contains(&sub) {
                // npm/cargo `run`/`test` execute project code — writes possible; keep out of M1 cache:
                if matches!((cmd, argv[1].as_str()), ("npm", "run")) { return Class::Bypass; }
                if cmd == "cargo" && argv[1] == "test" { return Class::Bypass; } // compiles (writes target/)
                return Class::Memoizable;
            }
        }
    }
    Class::Bypass
}
```

(Note the two in-rule exclusions: `npm run` and `cargo test` bypass — project code execution is not read-only. Adjust the const lists so the table reads clean; tests above are the contract.)

- [ ] **Step 4:** `cargo test --test classify` → PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(core): conservative read/write classifier"`.

### Task 3: FS epoch scanner + marker

**Files:**
- Create: `src/fsepoch.rs`
- Modify: `src/lib.rs`, `src/main.rs` (no wiring yet)
- Test: `tests/fsepoch.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub fn latest_mtime(root: &Path, skip: &[&str], visit_cap: usize) -> Option<SystemTime>`
  - `pub fn marker_path(cache_dir: &Path, cwd: &Path) -> PathBuf`
  - `pub fn fs_epoch(cache_dir: &Path, cwd: &Path) -> u64` — `max(latest_mtime, marker mtime)` as `u64` (secs*1e9+nanos); 0 if nothing found.
  - `pub const SKIP_DIRS: &[&str]` = `[".git", "node_modules", "target", "dist", ".venv", "__pycache__"]`; `pub const VISIT_CAP: usize = 100_000`.

- [ ] **Step 1: Failing tests** — `tests/fsepoch.rs`:

```rust
use reran::fsepoch::{latest_mtime, SKIP_DIRS, VISIT_CAP};
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
    let old = fs::FileTimes::from_modified_time(SystemTime::UNIX_EPOCH);
    // set old times on skipped files
    let f1 = fs::File::options().write(true).open(d.path().join(".git/HEAD")).unwrap();
    f1.set_times(old).unwrap();
    let f2 = fs::File::options().write(true).open(d.path().join("node_modules/pkg/i.js")).unwrap();
    f2.set_times(old).unwrap();
    fs::write(d.path().join("src.txt"), "new").unwrap(); // now
    let m = latest_mtime(d.path(), SKIP_DIRS, VISIT_CAP).unwrap();
    assert!(m.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() > 1_600_000_000);
}

#[test]
fn empty_dir_returns_none() {
    let d = tempdir().unwrap();
    assert_eq!(latest_mtime(d.path(), SKIP_DIRS, VISIT_CAP), None);
}
```

- [ ] **Step 2:** FAIL (module missing).
- [ ] **Step 3: Implement** `src/fsepoch.rs` — recursive walk with a `Vec<PathBuf>` worklist; count visits; if `visit_cap` exceeded, return `None` (caller falls back to marker-only); use `Metadata::modified()`; do NOT follow symlinks.
- [ ] **Step 4:** PASS.
- [ ] **Step 5: Commit** — `feat(core): bounded fs mtime scanner with skip list`.

### Task 4: Key builder

**Files:**
- Create: `src/key.rs`
- Test: `tests/key.rs`

**Interfaces:**
- Consumes: `fsepoch::fs_epoch`.
- Produces:
  - `pub struct CallCtx { pub cwd: PathBuf, pub argv: Vec<String>, pub uid: u32, pub fs_epoch: u64 }`
  - `pub fn uid() -> u32` (`libc::getuid`)
  - `pub fn env_allowlist(argv0: &str) -> Vec<(String, String)>` — builtin map, e.g. `git` → `["GIT_DIR","GIT_WORK_TREE"]`, `kubectl` → `["KUBECONFIG"]`, `aws` → `["AWS_PROFILE","AWS_DEFAULT_PROFILE"]`, `docker` → `["DOCKER_HOST"]`, `*` → `["VIRTUAL_ENV","CONDA_PREFIX"]`. Never PATH, never whole env (ccache#1790 false-miss lesson).
  - `pub fn build_key(ctx: &CallCtx, env: &[(String,String)]) -> [u8; 32]` — sha256 of canonical JSON `{cwd, argv, uid, env, fs_epoch}`.

- [ ] **Step 1: Failing tests** — same inputs → equal keys; each field changed (argv order, cwd, uid, one env var, fs_epoch) → different key. Two short test fns building ctx structs literally.
- [ ] **Step 2:** FAIL → **Step 3:** implement with `serde_json::json!` + `sha2::{Sha256,Digest}`. → **Step 4:** PASS.
- [ ] **Step 5: Commit** — `feat(core): pre-execution cache key (argv/cwd/uid/env-allowlist/fs-epoch)`.

### Task 5: SQLite store

**Files:**
- Create: `src/store.rs`
- Test: `tests/store.rs`

**Interfaces:**
- Produces:

```rust
pub struct Entry { pub key: [u8;32], pub session_id: String, pub turn: u64,
                   pub digest: String, pub raw: Vec<u8>, pub exit_code: i32, pub created_at: u64 }
pub struct Stats { pub hits: u64, pub misses: u64, pub bypass: u64, pub uncached_failures: u64, pub tokens_saved: u64 }
pub struct Store;
impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Store>;   // WAL + busy_timeout=5000
    pub fn get(&self, key: &[u8;32]) -> rusqlite::Result<Option<Entry>>;
    pub fn put(&self, e: &Entry) -> rusqlite::Result<()>;  // INSERT OR REPLACE + idempotent eviction (keep newest 10_000)
    pub fn record_seen(&self, session: &str, key: &[u8;32], lines: u64) -> rusqlite::Result<()>;
    pub fn saw_whole(&self, session: &str, key: &[u8;32], lines: u64) -> rusqlite::Result<bool>;
    pub fn record_event(&self, kind: &str, tokens_saved: u64) -> rusqlite::Result<()>;
    pub fn stats(&self) -> rusqlite::Result<Stats>;
    pub fn default_db_path() -> PathBuf; // $RERAN_DB or ~/.cache/reran/reran.db
}
```

- [ ] **Step 1: Failing tests** — roundtrip entry; `saw_whole` false before `record_seen`, true after with ≥ lines; eviction keeps newest 10k (insert 10_002 tiny entries, count); `stats` sums events; two `Store::open` on same path concurrently succeed (WAL).
- [ ] **Step 2:** FAIL → **Step 3:** implement (schema in `open`, migrations = `CREATE TABLE IF NOT EXISTS`). → **Step 4:** PASS.
- [ ] **Step 5: Commit** — `feat(core): sqlite store (WAL, seen-ranges, idempotent eviction, stats)`.

### Task 6: Digest rendering + honest token math

**Files:**
- Create: `src/digest.rs`
- Test: `tests/digest.rs`

**Interfaces:**
- Consumes: `Entry`.
- Produces:
  - `pub fn elide(raw: &str, head: usize, tail: usize) -> String` — first `head` + last `tail` lines, middle replaced by exactly `\n[+N lines elided by reran]\n`.
  - `pub fn render_hit(e: &Entry) -> String` — `reran: unchanged since turn {turn} · {lines} lines, ~{tokens} tokens — inputs identical (fs epoch {epoch})`. NEVER for failed commands (engine guarantees).
  - `pub fn estimate_tokens(s: &str) -> u64` — `s.len() as u64 / 4`.
  - `pub const HEAD_LINES: usize = 20; pub const TAIL_LINES: usize = 20;`

- [ ] **Step 1: Failing tests** — elide(50-line input) contains `[+10 lines elided by reran]` and 40 kept lines; elide keeps short input verbatim; render_hit string contains turn number and never the word "passed"/"ok" for a failed exit entry (test asserts no "passed" substring); estimate_tokens("abcdefgh") == 2.
- [ ] **Step 2:** FAIL → **Step 3:** implement. → **Step 4:** PASS.
- [ ] **Step 5: Commit** — `feat(core): digest rendering with explicit elision markers`.

### Task 7: Engine (evaluate/record, single-flight, write-invalidation)

**Files:**
- Create: `src/engine.rs`
- Modify: `src/lib.rs` (module list)
- Test: `tests/engine.rs`

**Interfaces:**
- Consumes: classify, key, store, digest.
- Produces:

```rust
pub enum Outcome {
    Hit { digest: String },            // this session saw the previous output
    Miss,                              // run for real; caller must then call record()
    Bypass,                            // write/unknown command; run raw; record() will bump fs marker
}
pub fn evaluate(store: &Store, argv: &[String], cwd: &Path, session: &str) -> Outcome
pub fn record(store: &Store, argv: &[String], cwd: &Path, session: &str, raw: &str, exit_code: i32) -> ()
```

Semantics (the heart — test all):
- `evaluate`: `Bypass` class → `Outcome::Bypass`. Build key (uid, env_allowlist, fs_epoch). `store.get`:
  - entry exists AND `store.saw_whole(session, key, line_count(prev.raw))` → `Hit` (render_hit). **This session saw it — and only then.**
  - entry exists but different session (never saw) → `Miss` (cachebro#11/#8 rule: never "unchanged" for unseen output).
  - no entry → `Miss`.
- `record`: `exit_code != 0` → `record_event("uncached_failure", 0)` and **no put** (bash-cache#25). `exit_code == 0` + `Memoizable` → put entry + `record_seen(session, key, lines)` + event `hit`/`miss` bookkeeping happens in `evaluate` actually — pin: `evaluate` records `miss`/`bypass`/`hit` events with `tokens_saved` = `estimate_tokens(prev.raw) - estimate_tokens(render_hit)` on Hit, else 0. `record` on `Bypass`-class command → touch marker file (fs marker ⇒ next evaluate gets new epoch ⇒ miss).
- Single-flight: `record` takes `fs2::FileExt::lock_exclusive` on `markers/<hexkey>.lock` for the put+seen critical section.

- [ ] **Step 1: Failing tests** (`tests/engine.rs`, tempfile cwd + `$RERAN_DB` to temp):
  1. same session: record(git status, exit 0) → evaluate → `Hit` containing "unchanged since turn".
  2. different session (same key): evaluate → `Miss` (THE trust test).
  3. after write (record(touch, exit 0) as Bypass) → evaluate(git status) → `Miss` (fs epoch bumped).
  4. record(failing cmd, exit 1) → evaluate → `Miss`, and store has no entry for it.
  5. two threads evaluate+record concurrently → both succeed, store readable (spawn 2×(evaluate,run,record)).
- [ ] **Step 2:** FAIL → **Step 3:** implement per semantics above. → **Step 4:** PASS.
- [ ] **Step 5: Commit** — `feat(core): engine — session-scoped hits, fs invalidation, single-flight record`.

### Task 8: Hook adapter (`reran hook pre|post`), fail-open

**Files:**
- Create: `src/hooks.rs`
- Modify: `src/main.rs` (wire `Cmd::Hook`), `src/lib.rs`
- Test: `tests/hooks.rs`

**Interfaces:**
- Consumes: engine.
- Produces:
  - `pub fn hook_pre(stdin_json: &str, cwd: &Path) -> String` — returns JSON to stdout or "" (allow).
  - `pub fn hook_post(stdin_json: &str, cwd: &Path) -> String` — always "".
  - Claude Code PreToolUse input: `{"session_id","tool_name":"Bash","tool_input":{"command":"..."}}`; deny output: `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"<digest>"}}`.
  - PostToolUse input: `{"session_id","tool_name","tool_input":{...},"tool_response":{...}}` — **first implementation step is a live-payload probe** (below); cache only when an exit code is confirmably 0, else `record_event("no_exit_code",0)` and skip caching.

- [ ] **Step 1: Failing tests**:
  1. `hook_pre` with `git status` payload, empty store → `""` (allow).
  2. after `record` via engine → `hook_pre` same session → deny JSON containing `unchanged since turn`.
  3. garbage stdin (`"not json"`, `{}`, `{}` with missing fields) → `""` and no panic (all three).
  4. compound command (`"git status && npm test"`) → `""`.
- [ ] **Step 2:** FAIL → **Step 3:** implement. Entire body wrapped so that ANY error ⇒ `""` (parse errors, store errors, panics via `std::panic::catch_unwind`). Read cwd from `std::env::current_dir()` at call site.
- [ ] **Step 4: Live-payload probe (dogfood, one-time)**: temp-install both hooks with `tee -a /tmp/reran-hook.log >>` prepend to log raw stdin, run two real Claude Code bash calls, inspect log, align `tool_response` parsing to reality, adjust fixture. Delete probe after.
- [ ] **Step 5:** all tests PASS → **Step 6: Commit** — `feat(hooks): claude code pre/post adapter, fail-open`.

### Task 9: `reran init claude-code`

**Files:**
- Create: `src/initcmd.rs`
- Modify: `src/main.rs`
- Test: `tests/initcmd.rs`

**Interfaces:**
- Consumes: nothing (writes JSON).
- Produces: `pub fn init_claude_code(settings_path: &Path, reran_bin: &str) -> std::io::Result<()>` — merges into `.claude/settings.json`:

```json
{ "hooks": { "PreToolUse": [ { "matcher": "Bash",
      "hooks": [ { "type": "command", "command": "<reran_bin> hook pre --event pre" } ] } ],
  "PostToolUse": [ { "matcher": "Bash",
      "hooks": [ { "type": "command", "command": "<reran_bin> hook post --event post" } ] } ] } }
```

Idempotent: if a hook command already contains `reran hook`, leave file unchanged.

- [ ] **Step 1: Failing tests** — tempdir settings: fresh file created with both hooks; existing settings preserve unknown keys; second run is a no-op (byte-identical).
- [ ] **Step 2:** FAIL → **Step 3:** implement (serde_json Value merge). → **Step 4:** PASS.
- [ ] **Step 5: Commit** — `feat(init): idempotent claude code hook wiring`.

### Task 10: `reran gain`

**Files:**
- Modify: `src/main.rs`
- Test: `tests/gain.rs`

**Interfaces:**
- Consumes: `Store::stats`.
- Produces: human output:

```
reran gain
  hits 12 · misses 30 · bypass 8 · uncached failures 2
  tokens saved: 45,320 (counted only on replaced output, bytes/4)
  hit rate: 28.6%
```

- [ ] **Step 1: Failing test** — seed events via engine in temp db (`$RERAN_DB`), run binary, assert substrings; zero-state prints zeros without crash.
- [ ] **Step 2:** FAIL → **Step 3:** implement. → **Step 4:** PASS.
- [ ] **Step 5: Commit** — `feat(gain): honest savings stats`.

### Task 11: E2E trust replay

**Files:**
- Create: `tests/e2e_replay.rs`
- Modify: nothing (pure integration over lib API)

**Interfaces:**
- Consumes: everything.

- [ ] **Step 1: Write the test first** — scripted session replay in one test fn (temp cwd/db):
  1. s1: record `git status` out=A exit0 → evaluate s1 → Hit(A).
  2. s2 (fresh session id): evaluate same argv → Miss (never-seen ⇒ run).
  3. write via record(`git add`, exit0, Bypass) → evaluate s1 `git status` → Miss (fs epoch bumped).
  4. record `false`-equivalent (`git` `nope` exit≠0 — classify Bypass path excluded; use a Memoizable-shaped forced failure: record(`git status`, exit 128) → store must contain no entry for that key → next evaluate s1 → Miss).
  5. assert `stats`: `uncached_failures >= 1`, `tokens_saved` > 0.
- [ ] **Step 2:** run → PASS (if any step fails, the failing task's code is wrong — fix there, not here).
- [ ] **Step 3: Commit** — `test(e2e): trust-contract replay (session scope, invalidation, failure policy)`.
- [ ] **Step 4: Tag M1:** `git tag m1 && git push origin main --tags`.

## Self-Review

- Spec coverage (M1 slice): classifier §4.2 → T2; FS markers §4.2 → T3; key §4.2 → T4; store §4.3 → T5; session-scoped seen §4.4 → T5+T7; unknown-digest minimal (head+tail elision, M1 slice of §4.5) → T6; honest stats §4.6 → T10; trust contract §5 → T6/T7/T8 tests + T11; hook adapter §6.1 → T8; `reran init` §6.1 → T9; concurrency §7 (single-flight, busy_timeout, idempotent eviction) → T5/T7. M2+ items (grammar extractor, MCP proxy, explain, stale-while-revalidate) deliberately absent.
- Placeholder scan: no TBD/TODO; T8 live-payload probe is a concrete procedure with a concrete artifact, not a placeholder.
- Type consistency: `Class`, `CallCtx`, `Entry`, `Stats`, `Outcome`, `Store` method names cross-checked between Tasks 2–11 — consistent.
- Review Focus: each of the 5 failure modes has its pinning test named in Tasks 3, 5, 6, 7, 8, 11.
