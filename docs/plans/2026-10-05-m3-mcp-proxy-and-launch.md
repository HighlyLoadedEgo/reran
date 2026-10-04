# M3 remainder + M4 launch plan (next session)

Status before this plan: M1+M2 shipped on main (74/74). M3 partial: explain done,
init claude-code + init zcode done. Probe installed (~/.cache/reran/probe-hook.sh,
fires in NEW sessions; log at ~/.cache/reran/probe.log).

## Task A: read the probe (USER-assisted, 5 min)
1. User opens any new ZCode session, runs 2 bash commands.
2. Read ~/.cache/reran/probe.log — inspect tool_response shape: does it carry
   exit_code/exitCode/code/status?
3. If exit confirmable → hook_post already caches; ZCode caching is LIVE.
   If not → decide M2.x: (a) wrap command in PreToolUse rewrite (permission-model
   risk, rtk#3152 class), or (b) accept dormant hook caching for ZCode, MCP-proxy
   becomes the primary channel. Remove probe hook from config after reading.

## Task B: MCP-proxy adapter (new crate src/bin or crates/)
- Transparent stdio proxy: wraps the harness's shell tool, per-connection session
  scope (connection id = session id), same core.
- Shape: `reran proxy -- <harness shell command>` OR MCP server exposing run_command
  with cache semantics; study rtk-mcp + toolcall-cache for the contract.
- Config: harnesses without hooks point their Bash tool at the proxy.
- Tests: fake stdio client e2e, per-connection isolation (cachebro#8 class).

## Task C: M4 publish (needs USER confirmation for each registry)
1. cargo: `cargo publish` (name reran verified free).
2. npm: create org scope or unscoped, publish wrapper + per-platform binaries
   built by scripts/release.sh (OIDC trusted publishing, user has the SOP skill).
3. Homebrew: tap HighlyLoadedEgo/tap, formula from release tarball + SHA256SUMS.
4. Benchmark: scripts/bench.sh — replay N recorded sessions through hook path,
   report tokens saved vs raw (methodology from spec §8; honest accounting).
5. Launch SOP: user's oss-launch-discoverability skill (GitHub Release, topics,
   awesome-list PR, MCP Registry).

## Non-goals still standing
Windows (v1.x), semantic matching (never), secrets filtering (hygiene only).
