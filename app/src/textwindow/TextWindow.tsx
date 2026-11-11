import { useCallback, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { HINT_READY, MAX_COMMAND_CHARS, createSubmitter, viewAfter } from "./command";
import "./text.css";

// The release "Type a command" window. It can do one thing: hand the typed
// text to the host, which turns it into a text event. Niva answers by voice, so
// nothing is shown back here.
export default function TextWindow() {
  const [text, setText] = useState("");
  const [failed, setFailed] = useState(false);

  // One command in flight at a time: a repeated Enter cannot send it twice.
  const submit = useMemo(() => createSubmitter(invoke), []);

  const send = useCallback(async () => {
    const view = viewAfter(await submit(text));
    if (view === null) return;
    if (view.clear) setText("");
    setFailed(view.failed);
  }, [submit, text]);

  return (
    <main className="text-window">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        <label htmlFor="command">Type a command</label>
        <div className="row">
          <input
            id="command"
            type="text"
            value={text}
            maxLength={MAX_COMMAND_CHARS}
            placeholder="Open Spotify, volume 30, mute..."
            onChange={(e) => setText(e.target.value)}
            autoFocus
            autoComplete="off"
            spellCheck={false}
          />
          <button type="submit">Send</button>
        </div>
        <p className="hint" role="status">
          {failed ? "Could not send the command. Try again." : HINT_READY}
        </p>
      </form>
    </main>
  );
}
