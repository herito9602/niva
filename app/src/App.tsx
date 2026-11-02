import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import "./styles.css";

type Phase = "hidden" | "in" | "out";
type Mode = "listening" | "thinking";

export default function App() {
  const [phase, setPhase] = useState<Phase>("hidden");
  const [mode, setMode] = useState<Mode>("listening");

  useEffect(() => {
    const subs = [
      listen("island:show", () => {
        setMode("listening");
        setPhase("in");
      }),
      listen("island:hide", () => setPhase("out")),
    ];
    return () => {
      subs.forEach((p) => p.then((un) => un()));
    };
  }, []);

  return (
    <div className="stage">
      <div
        className={`island ${phase} ${mode}`}
        onClick={() => setMode((m) => (m === "listening" ? "thinking" : "listening"))}
      >
        <div className="orb" />
        <span className="label">
          {mode === "listening" ? "Hey Niva — listening…" : "Thinking…"}
        </span>
      </div>
    </div>
  );
}
