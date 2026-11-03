import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// The debug window is a second window of the same bundle, opened by the host
// (debug builds only) at `index.html?window=debug`. It is imported lazily
// behind `import.meta.env.DEV` so production bundles never contain it.
const isDebugWindow =
  import.meta.env.DEV && new URLSearchParams(window.location.search).get("window") === "debug";

async function loadRoot(): Promise<React.ReactElement> {
  if (isDebugWindow) {
    const { default: DebugWindow } = await import("./debug/DebugWindow");
    return <DebugWindow />;
  }
  return <App />;
}

loadRoot().then((root) => {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>{root}</React.StrictMode>,
  );
});
