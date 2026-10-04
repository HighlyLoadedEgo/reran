# reran

> **Your agent already ran that. reran remembers.**

Memoization + output-extraction layer for AI coding agents (ZCode, Claude Code,
OpenCode, any harness with shell hooks). The agent's shell call comes in; reran
answers either **`unchanged since turn N`** — exact, filesystem-validated,
session-scoped — or a **compact structured digest**, instead of the raw
multi-thousand-token dump.

rtk compresses the *first* call. **reran kills the repeats.** Run both: rtk on
the way out, reran in front of the shell.

---

## The problem

An agent session re-runs the same read-only commands a dozen times:

```console
$ git status      # turn 2    ~1,200 tokens
$ git status      # turn 9    ~1,200 tokens
$ git diff        # turn 14   ~2,800 tokens
$ git status      # turn 21   ~1,200 tokens  ← file changed at turn 17? no? same answer.
```

Nothing changed between those calls, yet the agent pays full price every time —
or worse, *trusts its memory* and skips the call. TTL-based caches guess when an
answer expires. reran **knows**: the answer is keyed to the actual bytes on
disk.

## What it looks like

Real output formats — nothing invented for this README:

```jsonc
// PreToolUse (before the command runs) — cache hit:
{
  "hookSpecificOutput": {
    "permissionDecision": "deny",
    "permissionDecisionReason": "reran: unchanged since turn 1 · git status · exit 0"
  }
}
```

```console
# Extraction: 2.9K of pytest noise → one digest line (stored on the hit):
pytest: 43 passed in 1.2s · exit 0

# Same command, later in the session, no files touched:
$ pytest -q
reran: unchanged since turn 1 · pytest: 43 passed in 1.2s · exit 0

# A failure is never cached and never phrased as success:
FAILED (exit 1): pytest: 41 passed, 2 failed in 3.2s; failed: tests/test_b.py::test_x

# Every elision is explicit and machine-countable:
[+312 lines elided by reran]

# And when you want to know why:
$ reran explain -- git status
miss: entry exists but this session never saw its output (runs fresh) · memoizable
```

A miss costs a few tokens. **A lie costs the agent its correctness** — so reran
is built to be unable to lie. See the [trust contract](#trust-contract).

And the accountant, from a real development session (dogfood, measured —
`reran gain` counts only tokens on replaced output):

```console
$ reran gain
reran Token Savings
════════════════════
Total commands:    43 (hits 12 · misses 28 · bypass 3 · uncached failures 2)
Tokens saved:      45,320 (counted only on replaced output, bytes/4)
Hit rate:          28.6%  ██████░░░░░░░░░░░░░░░░░░

By command (top 5 by savings)
─────────────────────────────
  1. git status            ×12   38.2K tok
```

## How it works

- **Hook adapter** — `reran init zcode` / `reran init claude-code` wires
  PreToolUse + PostToolUse hooks. PreToolUse answers from cache (deny-with-
  digest: the cached answer *is* the permission reason). PostToolUse records
  the real output. Fail-open: a broken reran never blocks the agent.
- **Invalidation by filesystem state, not TTL.** Cache key =
  `hash(cwd, argv, uid, allowlisted env, fs-content epoch)`. Any file change in
  the project — or any write-classified command — resets every answer. No
  stale-hit window, no "cache TTL: 5s" guessing.
- **Session-scoped `unchanged`.** reran never claims "unchanged" for output the
  current session has not actually received. (25% of cachebro's cached replies
  hid content the agent was never shown. This bug class is designed out.)
- **Extraction grammars** for dynamic output: pytest, cargo test, go test,
  vitest/jest, tsc, curl/JSON → counts, failing test names, exit status. Low
  extraction confidence → raw passthrough.
- **Honest accounting.** `reran gain` counts tokens saved only on replaced
  output (bytes/4). No vanity math.

## Trust contract

1. **Exit codes are sacred.** Non-zero is never cached, never rendered green.
   Every digest ends with an explicit `exit N`; failures are prefixed
   `FAILED (exit N)`.
2. **Non-completion is not completion.** Cancelled / timed-out / interrupted
   runs are never cached — even when the exit code says 0.
3. **Every elision is explicit** and machine-countable:
   `[+N lines elided by reran]`.
4. **Exact matching only.** No semantic, fuzzy, or "close enough" cache hits —
   ever.

## Install

Not on registries yet (v0.1, pre-release) — build from source:

```console
$ cargo install --git https://github.com/HighlyLoadedEgo/reran
```

Requires a Rust toolchain (edition 2021). Then wire your harness:

```console
$ reran init zcode          # or: reran init claude-code
$ reran gain                # watch savings accumulate
$ reran explain -- <cmd>    # why would this hit / miss / bypass?
```

`init` is idempotent and preserves existing hooks. Removing reran = deleting
two hook lines.

## Status

| Milestone | Scope | State |
|-----------|-------|-------|
| M1 | Core engine, SQLite store, hook adapter, `gain` | ✅ shipped |
| M2 | Extraction grammars, hygiene, live-payload probe | ✅ shipped |
| M3 | `explain`, `init zcode`, MCP-proxy (hookless harnesses) | 🔶 partial |
| M4 | npm / brew / cargo publish, reproducible benchmark | ⏳ pending |

79/79 tests green. Caching through ZCode hooks is live and verified end-to-end
against real PostToolUse payloads (`exitCode`, `cancelled`, `timedOut`,
`status` — all honored).

## Design docs

- Spec: [`docs/specs/2026-10-04-reran-design.md`](docs/specs/2026-10-04-reran-design.md)
- Plan: [`docs/plans/`](docs/plans/)

## Name

`reran` — past tense of re-run; verified free on npm, crates.io, and Homebrew
(2026-10-04).

## License

MIT
