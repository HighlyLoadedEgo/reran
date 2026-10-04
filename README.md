# reran

> Your agent already ran that. reran remembers.

Token-optimizing memoization + output-extraction layer for AI coding agents
(Claude Code, ZCode, OpenCode…). The agent's shell call comes in; reran answers
either `unchanged since turn N` (exact, FS-content-validated, session-scoped)
or a compact structured digest — instead of the raw multi-thousand-token dump.

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

- **Hook adapter** (`reran init claude-code|zcode`): PreToolUse answers from cache
  (deny-with-digest), PostToolUse records real output. Fail-open: a broken reran
  never blocks the agent.
- **Invalidation by filesystem state**, not TTL: any file change in the project
  or any write-classified command resets the answer. Key = cwd + argv + uid +
  allowlisted env + fs epoch.
- **Session-scoped "unchanged"**: reran never says "unchanged" for output the
  agent has not seen in THIS session (cachebro#11 class bug — 25% of their
  cached replies hid never-delivered content — is designed out).
- **Failures are never cached**: exit code is sacred and printed in every digest.
- **Extraction grammars** (M2): pytest, cargo test, go test, vitest/jest, tsc,
  curl/JSON → compact digests with counts, failing test names, exit status.
- **Explain**: `reran explain -- git status` tells you exactly why it would
  hit, miss, or bypass (doit#329, the 7-year-old unmet demand).

## Trust contract

1. Exit codes are sacred — non-zero is never cached, never rendered green.
2. Every elision is explicit and machine-countable (`[+N lines elided by reran]`).
3. Low extraction confidence → raw passthrough. A miss costs tokens; a lie
   costs the agent's correctness.
4. Exact matching only — no semantic/fuzzy cache hits, ever.

## Status

- **M1 shipped** (this branch): core engine + SQLite store + hook adapter +
  `gain`/`explain` + honest stats.
- **M2 shipped**: extraction grammars, events/lock hygiene, live-payload probe
  installed.
- **M3 in progress**: MCP-proxy adapter (for harnesses without hooks) — see
  `docs/plans/`.
- **M4 pending**: npm/brew/cargo distribution, reproducible benchmark.

Caching via hooks activates once the harness's PostToolUse payload carries a
confirmable exit code (probe: `~/.cache/reran/probe.log`).

## Name

`reran` — verified free on npm / crates.io / Homebrew (2026-10-04).

## License

MIT
