// Fails if the release binary contains any trace of the debug command, of the
// debug window and its Accept path (app/src-tauri/src/debug.rs), or of the
// debug-only dev adapters (the dev LLM and the dry-run PC, search and
// reminders; app/src-tauri/src/dev.rs), or if the evaluation tooling can reach
// the app build: the `testing` fakes of niva-core and the YAML reader are
// behind the `eval` feature of the app (PR 2c) and must not be in its graph.
// The real Ollama adapter is legitimately in the release since PR 2d-a, so its
// strings (address, endpoint, model, log tag) are NOT needles any more.
// Usage: node scripts/check-release-binary.mjs [path-to-niva.exe]
import { spawnSync } from "node:child_process";
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
  // The debug window and its Accept path. Accept and Reject may only come from
  // a native click or a spoken answer in a release build; the debug window's
  // Confirm button is a Channel::Click and is safe only over the dry-run PC.
  // These are the window's title and URL, its prompt text and the serde names
  // of its debug-only messages.
  "Niva debug",
  "window=debug",
  "Close Notepad?",
  "set_media",
  "countdown_cancel",
  "countdown_fault",
  // The eval runner and the test fakes.
  "niva-eval",
  "fake script exhausted",
];
const found = needles.filter((n) => text.includes(n));
if (found.length > 0) {
  console.error(`release binary ${exe} contains: ${found.join(", ")}`);
  process.exit(1);
}
// The commands of the release webviews must be in it: without submit_text the
// tray entry would open a window that cannot send anything; without the island's
// two the confirmation buttons would be dead (PR 2d-b).
const required = ["submit_text", "answer_confirmation", "report_confirm_buttons"];
const missing = required.filter((n) => !text.includes(n));
if (missing.length > 0) {
  console.error(`release binary ${exe} is missing: ${missing.join(", ")}`);
  process.exit(1);
}

// Feature graph of the app build (no `--features`): the fakes of niva-core
// (`testing`) and the eval-only YAML reader must not be in it.
function cargoTree(args) {
  const run = spawnSync(process.platform === "win32" ? "cargo.exe" : "cargo", ["tree", "-p", "niva", ...args], {
    cwd: resolve(here, "../.."),
    encoding: "utf8",
  });
  if (run.error) {
    console.error(`could not run cargo to check the feature graph: ${run.error.message}`);
    process.exit(1);
  }
  return run;
}
const core = cargoTree(["-e", "features", "-i", "niva-core"]);
if (core.status !== 0 || core.stdout.includes('feature "testing"')) {
  console.error('the app build enables niva-core\'s "testing" feature (the fakes):');
  console.error(core.stdout || core.stderr);
  process.exit(1);
}
// Fail closed: the only accepted outcome is cargo saying the package is not in
// the graph. Any other failure (cargo missing, a broken manifest) must not pass.
const yaml = cargoTree(["-i", "serde_norway"]);
if (yaml.status === 0) {
  console.error("the app build depends on serde_norway (eval-only):");
  console.error(yaml.stdout);
  process.exit(1);
}
if (!yaml.stderr.includes("did not match any packages")) {
  console.error("could not confirm that serde_norway is absent from the app build:");
  console.error(yaml.stderr);
  process.exit(1);
}
console.log(
  `ok: ${exe} has no debug command, dev adapter or eval strings, and the app feature graph has no "testing" fakes`,
);
