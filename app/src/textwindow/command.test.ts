import { describe, expect, it } from "vitest";
import capability from "../../src-tauri/capabilities/text-window.json";
import windowSource from "./TextWindow.tsx?raw";
import {
  HINT_READY,
  MAX_COMMAND_CHARS,
  SUBMIT_COMMAND,
  createSubmitter,
  prepareCommand,
  viewAfter,
} from "./command";

/** An invoke whose calls stay pending until the test settles them. */
function controlledInvoke() {
  const calls: { cmd: string; args: unknown; resolve: () => void; reject: (e: unknown) => void }[] = [];
  const invoke = (cmd: string, args?: unknown) =>
    new Promise<unknown>((resolve, reject) => {
      calls.push({ cmd, args, resolve: () => resolve(undefined), reject });
    });
  return { invoke, calls };
}

describe("createSubmitter: one command in flight at a time", () => {
  it("sends the prepared text to submit_text and reports it", async () => {
    const { invoke, calls } = controlledInvoke();
    const submit = createSubmitter(invoke);
    const result = submit("  open notepad ");
    expect(calls).toHaveLength(1);
    expect(calls[0].cmd).toBe(SUBMIT_COMMAND);
    expect(calls[0].args).toEqual({ text: "open notepad" });
    calls[0].resolve();
    expect(await result).toBe("sent");
  });

  it("ignores a repeated Enter while the first command is still being sent", async () => {
    const { invoke, calls } = controlledInvoke();
    const submit = createSubmitter(invoke);
    const first = submit("volume 30");
    expect(await submit("volume 30")).toBe("busy");
    expect(await submit("volume 30")).toBe("busy");
    expect(calls).toHaveLength(1);
    calls[0].resolve();
    expect(await first).toBe("sent");
  });

  it("accepts the next command once the previous one finished", async () => {
    const { invoke, calls } = controlledInvoke();
    const submit = createSubmitter(invoke);
    const first = submit("mute");
    calls[0].resolve();
    await first;
    const second = submit("unmute");
    expect(calls).toHaveLength(2);
    calls[1].resolve();
    expect(await second).toBe("sent");
  });

  it("releases the guard after a failure so the user can try again", async () => {
    const { invoke, calls } = controlledInvoke();
    const submit = createSubmitter(invoke);
    const first = submit("mute");
    calls[0].reject(new Error("dispatcher is gone"));
    expect(await first).toBe("failed");
    const retry = submit("mute");
    expect(calls).toHaveLength(2);
    calls[1].resolve();
    expect(await retry).toBe("sent");
  });

  it("never calls the host for a blank command", async () => {
    const { invoke, calls } = controlledInvoke();
    const submit = createSubmitter(invoke);
    expect(await submit("   ")).toBe("blank");
    expect(calls).toHaveLength(0);
  });
});

describe("typing a command", () => {
  it("typed_command_enter_submits_and_clears_box", async () => {
    const { invoke, calls } = controlledInvoke();
    const submit = createSubmitter(invoke);
    const sending = submit("  open notepad ");
    calls[0].resolve();
    expect(await sending).toBe("sent");
    expect(calls[0].args).toEqual({ text: "open notepad" });
    expect(viewAfter("sent")).toEqual({ clear: true, failed: false });
  });

  it("the hint says Niva answers by voice and names no island text", () => {
    expect(HINT_READY).toMatch(/voice/i);
    expect(HINT_READY).not.toMatch(/island|screen|appears/i);
  });
});

describe("viewAfter: what the box shows after a submit", () => {
  it("clears the box and the error hint after a successful send", () => {
    expect(viewAfter("sent")).toEqual({ clear: true, failed: false });
  });

  it("keeps the text and shows the error hint after a failure", () => {
    expect(viewAfter("failed")).toEqual({ clear: false, failed: true });
  });

  it("changes nothing for a blank command or a repeated Enter", () => {
    expect(viewAfter("blank")).toBeNull();
    expect(viewAfter("busy")).toBeNull();
  });
});

describe("prepareCommand", () => {
  it("trims the text and drops a blank one", () => {
    expect(prepareCommand("  open notepad \n")).toBe("open notepad");
    expect(prepareCommand("   ")).toBeNull();
    expect(prepareCommand("")).toBeNull();
  });

  it("caps the text at the host's limit, in characters", () => {
    expect(MAX_COMMAND_CHARS).toBe(500);
    const long = "ñ".repeat(MAX_COMMAND_CHARS + 40);
    expect(Array.from(prepareCommand(long) ?? "")).toHaveLength(MAX_COMMAND_CHARS);
  });

  it("keeps the text a user typed as it is", () => {
    expect(prepareCommand("volume 30")).toBe("volume 30");
    expect(prepareCommand("sí, abrí Spotify")).toBe("sí, abrí Spotify");
  });
});

describe("the window's reach", () => {
  it("calls the one command its capability grants", () => {
    expect(SUBMIT_COMMAND).toBe("submit_text");
    expect(capability.permissions).toEqual(["allow-" + SUBMIT_COMMAND.replace("_", "-")]);
  });

  it("sends only through the one-at-a-time submitter, never a bare invoke", () => {
    expect(windowSource).toMatch(/createSubmitter\(invoke\)/);
    expect(windowSource).not.toMatch(/await invoke\(/);
  });

  it("uses no host API but invoke: no events, no window control", () => {
    const imports = [...windowSource.matchAll(/from\s+"([^"]+)"/g)].map((m) => m[1]);
    expect(imports.filter((name) => name.startsWith("@tauri-apps"))).toEqual(["@tauri-apps/api/core"]);
    expect(windowSource).not.toMatch(/emit\(|listen\(|getCurrentWindow|debug_dispatch/);
  });
});
