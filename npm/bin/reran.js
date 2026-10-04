#!/usr/bin/env node
// reran npm wrapper: exec the platform binary from optionalDependencies.
const { spawnSync } = require("node:child_process");
const { createRequire } = require("node:module");
const require = createRequire(__filename);
const platform = `${process.platform}-${process.arch}`;
let bin;
try {
  bin = require(`@reran/${platform}/bin/reran`);
} catch {
  console.error(`reran: no binary for ${platform} (expected in optionalDependencies)`);
  process.exit(1);
}
const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
process.exit(r.status ?? 1);
