// Fails if the release binary contains any trace of the debug command or of
// the debug-only dev adapters (the dev LLM and the dry-run PC, search and
// reminders; app/src-tauri/src/dev.rs).
// Usage: node scripts/check-release-binary.mjs [path-to-niva.exe]
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const exe = process.argv[2] ?? resolve(here, "../../target/release/niva.exe");
const text = readFileSync(exe).toString("latin1");
const needles = [
  // The debug command and its permission.
  "debug_dispatch",
  "debug-dispatch",
  // Markers in every log line of the dev LLM and of the dry-run adapters.
  "niva-dev-llm",
  "niva-dry-run",
  // Other dev-only strings: canned replies and the debug window's log event.
  "dev assistant",
  "Dev summary",
  "niva://dev-log",
];
const found = needles.filter((n) => text.includes(n));
if (found.length > 0) {
  console.error(`release binary ${exe} contains: ${found.join(", ")}`);
  process.exit(1);
}
console.log(`ok: ${exe} has no debug command or dev adapter strings`);
