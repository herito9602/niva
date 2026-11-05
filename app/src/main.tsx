import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// Extra windows are further windows of the same bundle, opened by the host at
// `index.html?window=<name>`.
//
// - `text` is the release "Type a command" window (tray entry).
// - `debug` is the debug window (debug builds only). It is imported lazily
//   behind `import.meta.env.DEV` so production bundles never contain it.
const windowName = new URLSearchParams(window.location.search).get("window");
const isDebugWindow = import.meta.env.DEV && windowName === "debug";
const isTextWindow = windowName === "text";

async function loadRoot(): Promise<React.ReactElement> {
  if (isDebugWindow) {
    const { default: DebugWindow } = await import("./debug/DebugWindow");
    return <DebugWindow />;
  }
  if (isTextWindow) {
    const { default: TextWindow } = await import("./textwindow/TextWindow");
    return <TextWindow />;
  }
  return <App />;
}

loadRoot().then((root) => {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>{root}</React.StrictMode>,
  );
});
