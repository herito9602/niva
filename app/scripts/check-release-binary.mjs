// Fails if the release binary contains any trace of the debug command.
// Usage: node scripts/check-release-binary.mjs [path-to-niva.exe]
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const exe = process.argv[2] ?? resolve(here, "../../target/release/niva.exe");
const text = readFileSync(exe).toString("latin1");
const needles = ["debug_dispatch", "debug-dispatch"];
const found = needles.filter((n) => text.includes(n));
if (found.length > 0) {
  console.error(`release binary ${exe} contains: ${found.join(", ")}`);
  process.exit(1);
}
console.log(`ok: ${exe} has no debug command strings`);
