# reran — design spec (v1)

- **Date**: 2026-10-04
- **Status**: draft for review
- **License**: MIT (assumption, flag if wrong)
- **Repo**: `reran` (name verified free on npm / crates.io / Homebrew, 2026-10-04)

## 1. Problem

Bash is the dominant token cost in coding-agent sessions (user's own spend chart:
Bash ≈ $75 of ~$120 attributed). Static output filtering (rtk) has hit its
ceiling: measured 9.6% savings, 95% of it from a single tool (`rtk read`), while
curl/JSON/unknown commands leak at ~0% (34.2M tokens in rtk#4390).

Two untapped levers:

1. **Repeated calls** — agents re-check state (`git status`, `ls`, re-runs).
   Each call = a full request with growing context.
2. **Unknown dynamic output** — test runners, builds, curl pass through raw.

Competitor check (2026-10-04): the niche "exact, FS-invalidated shell
memoization + extraction, drop-in for coding agents" is empty. Nearest neighbors:
cachebro (220★, file reads only, disk-hash invalidation with a correctness hole —
cachebro#11), claude-dejavu (abandoned), bkt (no agent integration).

## 2. Product definition

**reran** sits between the agent and the shell. For every shell call it returns
one of:

- `unchanged since turn N` (+ last digest) — when inputs provably identical
  AND the agent already saw the previous result;
- a **structured digest** (~50 tokens) — for outputs with a known grammar;
- **raw passthrough** — when confidence is low. Never silently mangle.

Formula vs rtk: *rtk compresses the first call; reran eliminates repeat calls
and digests unknown ones.* Complementary, not competing.

## 3. Non-goals (v1)

- No semantic/fuzzy cache matching (GPTCache died of false hits: 0.83–0.99
  cosine on negated queries). Exact hashing only, no exceptions.
- No secrets filtering priority (zero demand found in any tracker; hygiene later).
- No compaction-amnesia pinning / loop circuit breakers (proxy-layer scope, v2+).
- No prompt-cache warming (different layer; cache-tax cluster exists).
- No native Windows in v1 (macOS/Linux; Windows v1.x — known pain swamp:
  rtk MSYS saga, cachebro#9 WAL issues).
- Not a task runner / not doit-style declared pipelines.

## 4. Architecture

Rust binary `reran` + two thin adapters over one core library:

```
┌─ hook adapter ──┐   ┌─ MCP proxy adapter ──┐
│ PreToolUse      │   │ stdio/HTTP proxy      │
│ PostToolUse     │   │ (harnesses w/o hooks) │
└──────┬──────────┘   └──────┬───────────────┘
       └───────┬─────────────┘
         ┌─────▼─────┐
         │  core lib  │  classifier → key-builder → store → extractor
         └─────┬─────┘
         ┌─────▼─────┐
         │ SQLite WAL │  (per-user cache dir; multi-process safe)
         └───────────┘
```

### Components

1. **Classifier** — pre-execution decision: `memoizable` (read-only, deterministic),
   `extractable` (known grammar, run but digest), `bypass` (write/side-effectful:
   run raw; resets FS marker for its scope). Conservative: unknown → extractable-at-worst.
   v1.1: **read-only pipelines are memoizable** — agents compose nearly every
   read (`ls && echo --- && find … | head`). A pipeline is memoizable iff every
   `|`/`&&`/`;`-separated segment is; the whole composed argv is the cache key.
   fd-redirects (`2>&1`, `2>/dev/null`) touch no files and are safe; file
   redirects (`>`, `>>`, `2> path`) stay bypass. Operators glued inside a token
   (`2>&1|head`, no spaces) are invisible to segmentation → bypass (refuse
   rather than misread). Any write/unknown segment poisons its pipeline.
   v1.2: **sed** print-to-stdout forms are memoizable — every in-place form
   (`-i`, glued `-i.bak`/`-in`, `--in-place`) bypasses; the `w` script-command
   is a known residual gap. **tsc** is memoizable ONLY with `--noEmit` and
   without `--watch`/`-w`/`--incremental`/`--build`. Pure-stdout text
   utilities join the whitelist (jq, cut, uniq, wc, diff, stat, realpath,
   dirname, basename, column, xxd, checksums, seq, base64, tree, …); `sort`
   is deliberately absent (`-o file` writes).
   v1.3: **`cd DIR && <reads>`** — the cd segment is neutral iff DIR resolves
   inside the hook cwd subtree (fs_epoch scans exactly that subtree; a cd
   outside would read from a zone no write can invalidate). Bare `cd` caches
   nothing; without a cwd context cd is refused.
   **Opt-in test cache** (`reran allow-tests on`, per-cwd flag file): pytest /
   `python -m pytest` / `uv run pytest` / `cargo test` / `npm test` / `npx
   jest|vitest` become memoizable. Default OFF — tests may be
   nondeterministic; opting in is an explicit per-project decision.
   `.pytest_cache` / `.ruff_cache` / `.mypy_cache` joined fs-scan skip list:
   test-cache churn must not invalidate reads.
   v1.4: **marker bump only for plausible writers.** Bypass commands used to
   bump the cwd marker unconditionally ("unknown — might write"), and with
   agents running read-only diagnostics between reads (gh run list, kubectl
   get, sleep N; curl) the marker churned every second — repeated reads
   forced fresh misses, hits structurally impossible. bumps_fs_marker():
   bypass command skips the bump iff EVERY segment is a known non-writer
   (pure-stdout utilities, sed/tsc read forms, gh/kubectl/helm/sleep/ps/lsof
   minus gh repo|codespace|extension|auth); redirects, substitution, unknown
   and project-executing segments bump as before. Omissions only over-bump.
2. **Key-builder** — computes everything the output depends on, BEFORE execution
   (bkt#20 rule: key must be computable pre-run):
   `hash(cwd, argv, resolved interpreter, identity(uid), env-delta of an
   allowlist of env vars (per command class, user-extendable — e.g. KUBECONFIG,
   AWS_PROFILE; never the whole environment: over-keying causes false misses,
   ccache#1790), FS content markers)`.
   FS markers: `find -newer <marker>` on the command's cwd subtree; marker file
   updated on write-classified commands. No mtime-only keys (claude-dejavu's
   weakness), no disk-hash-only keys (cachebro#11's weakness).
3. **Store** — SQLite WAL in `~/.cache/reran/`; entries: key, session-scope,
   turn-id, output digest, raw-output ref (for lossless mode), exit code,
   seen-ranges per session. Atomic swap + idempotent eviction (bkt#38 lesson).
4. **Session-scope tracker** — the differentiator (cachebro#11, unclaimed):
   "unchanged" answers only for ranges the CURRENT session actually received.
   Hook payload `session_id` (Claude Code/ZCode provide it) or MCP connection id.
   Never-seen ranges → digest served fresh, never a bare "unchanged".
5. **Extractor** — grammar set (v1): test runners pytest/go test/cargo test/
   vitest/jest/tsc/eslint/ruff (exit-code-first: non-zero exit NEVER renders
   green — rtk#4421 class), curl/JSON (rtk#4390's biggest leak), cloud CLIs
   (aws/kubectl/gcloud) later in v1.x. Unknown → head+tail+error-lines with
   explicit `[+N lines elided]` markers. `jc`-style grammar registry, community-extensible.
6. **Stats + explain** — `reran gain` (tokens saved, calls avoided, hit rate —
   counted ONLY on actually-processed output, honest accounting per rtk#2001/#3943);
   `reran explain <query>` — why hit/miss/bypass: key hash, which input changed,
   age (doit#329: 7-year unmet demand; bkt#19 TSV schema as logging format).

### Concurrency & freshness

- Single-flight per key (flock) — parallel identical calls coalesce (bkt#11).
- Non-zero exit codes: not cached by default (configurable; bash-cache#25).
- Optional stale-while-revalidate: serve stale instantly, refresh in fully
  detached child (close std fds — bkt#60 hang lesson). Off by default in v1.
- TTL as belt-and-suspenders only; FS-content invalidation is the primary mechanism.

## 5. Trust contract (first-class, not a mode)

1. Exit codes are sacred: always propagated verbatim.
2. Never render failure as success (test-runner false-green class = top rtk bug class).
3. Every elision is explicit and machine-countable (`[+N lines elided]`).
4. Low extraction confidence → raw passthrough. A miss costs tokens; a lie costs the agent's correctness.
5. `--lossless` global mode: digest + pointer to full raw output retrievable via `reran show <id>`.
6. Hook adapter must not break harness permission models (rtk#3152/#4281 class):
   rewrite only, never re-order or inject commands; fail-open to raw on any internal error.

## 6. Adapters

1. **Hook adapter** (primary, dogfood channel): Claude Code + ZCode
   PreToolUse/PostToolUse wiring via one `reran init <harness>` command.
   Hooks-first because agents don't voluntarily call cache tools (cachebro#7).
2. **MCP proxy** (secondary): transparent stdio proxy wrapping the harness's
   shell tool; per-connection session scope. Same core, separate crate/binary.

**Stable adapter contract**: the hook/proxy interface is versioned and documented
(rtk#3366 lesson) so the community can port to kiro/OpenCode/JetBrains without
hand-rolling.

## 7. Distribution (day one, bkt#12 lesson)

`npm i -g reran` (native binary via optionalDependencies per-platform, provenance
via OIDC trusted publishing), `brew install reran`, `cargo install reran`.
All three names verified free. GitHub Releases carry a `.mcpb` bundle.

## 8. Testing

- Unit: classifier/key-builder/extractor grammars (golden files).
- **Replay harness**: recorded real session traces (like cachebro#11's filer —
  22 replayed sessions) run through core; assert no unseen-range "unchanged",
  no false-green, exit-code fidelity. This is the trust CI gate.
- e2e: hook wiring in a sandboxed Claude-Code-compatible harness; MCP proxy
  against a fake stdio client.
- Fuzz: extractor on random/adversarial output (never panic; passthrough on parse
  failure).

## 9. Milestones

- **M1** — core lib + key-builder + SQLite store + hook adapter (Claude Code),
  `unchanged` semantics with session-scoped seen-tracking, bypass on writes.
  Dogfood on own sessions; `reran gain` from day one.
- **M2** — extractor grammars (test runners, curl/JSON, unknown head+tail),
  trust contract enforcement, replay harness in CI.
- **M3** — MCP proxy adapter, `reran explain`, single-flight,
  `reran init` multi-harness (ZCode, OpenCode).
- **M4** — launch: npm/brew/cargo, README with honest benchmark methodology
  (published, reproducible — credibility per rtk#2001), awesome-list PR,
  MCP Registry, launch SOP.

## 10. Open questions (none blocking spec)

- Repo under personal account vs an org (rtk-ai?) — owner's call at launch.
- Extraction grammar registry format: embed jc's? (license MIT, compatible) — decide in M2.
