#!/usr/bin/env bash
# Mechanism benchmark: replay a synthetic read-heavy workload through the real
# hook path (temp DB, isolated) and report raw-vs-cached token cost.
# This measures the HIT MECHANISM, not "your session" — numbers are honest for
# the mechanism only. Run from anywhere.
set -euo pipefail
BIN="${RERAN_BIN:-$(command -v reran || echo "$HOME/.local/bin/reran")}"
export RERAN_DB="$(mktemp -d)/bench.db"

python3 - "$BIN" <<'EOF'
import json, subprocess, sys, tempfile, os, time

bin_path = sys.argv[1]
workdir = tempfile.mkdtemp(prefix="reran-bench-cwd-")

# Synthetic workload: 20 distinct read commands with fat deterministic outputs
# (the read-research shape: N commands, then M repeats of each).
commands = [f"cat docs/file{i}.md" for i in range(20)]
outputs = [f"# file{i}\n" + ("lorem ipsum dolor sit amet " * 400) for i in range(20)]
REPEATS = 3  # each command is re-run this many times after the first miss

def payload(cmd, stdout, session, call_id):
    return json.dumps({
        "session_id": session, "tool_name": "Bash", "cwd": workdir,
        "toolCallId": call_id,
        "tool_input": {"command": cmd},
        "tool_response": {"stdout": stdout, "stderr": "", "exitCode": 0,
                          "status": "completed", "cancelled": False, "timedOut": False},
    })

def hook(event, p):
    r = subprocess.run([bin_path, "hook", "--event", event], input=p,
                       capture_output=True, text=True)
    return r.stdout

def tokens(s): return len(s.encode()) // 4

session = "bench-1"
raw_tokens = 0        # what the agent would pay with NO cache
saved_tokens = 0      # what gain-style accounting credits (bytes/4 on replaced output)
hits = 0
t0 = time.time()
call = 0
for cmd, out in zip(commands, outputs):
    for rep in range(REPEATS + 1):
        call += 1
        p = payload(cmd, out, session, f"call_{call}")
        if rep == 0:
            hook("pre", p)          # miss
            hook("post", p)         # caches
            raw_tokens += tokens(out)
        else:
            deny = hook("pre", p)
            hook("post", p)
            raw_tokens += tokens(out)
            if '"permissionDecision": "deny"' in deny.replace(": ", ": ") or '"permissionDecision":"deny"' in deny:
                hits += 1
                digest = json.loads(deny)["hookSpecificOutput"]["permissionDecisionReason"]
                saved_tokens += tokens(out) - tokens(digest)

elapsed = time.time() - t0
n = len(commands) * (REPEATS + 1)
print("reran mechanism benchmark")
print("════════════════════════")
print(f"workload:            {len(commands)} reads × {REPEATS+1} runs = {n} calls")
print(f"elapsed:             {elapsed*1000:.0f} ms ({elapsed/n*1000:.1f} ms/hook avg)")
print(f"hits:                {hits}/{n - len(commands)} repeats")
print(f"raw cost (no cache): {raw_tokens:,} tok")
print(f"saved (bytes/4):     {saved_tokens:,} tok")
print(f"note: mechanism-only — a hit also KEEPS the output out of context,")
print(f"which compounds over every later turn of the session.")
EOF
