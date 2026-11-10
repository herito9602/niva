# Niva app

Tauri 2 desktop app (React + TypeScript + Vite) with a floating "island" window for the Niva assistant.

## Run

Run cargo and Tauri from PowerShell, never Git Bash (Git's `link` shadows MSVC `link.exe`).

```
npm install
npm run tauri dev
```

Tests: `npm test` (vitest) here, `cargo test` at the workspace root.

Voice: the app starts the speech sidecar of `../voice` by itself (debug and release alike).
Without it, or with `NIVA_VOICE=off`, Niva works in text mode. Setup, variables and the
command to try it by voice are in `../voice/README.md` ("Run it with the Niva host").

## Use

- Ctrl+Shift+Space toggles the island (starts hidden). Tray icon: "Show / hide island", "Type a command", "Restart voice", "Quit"; double-click shows it. The tooltip says what the voice is doing.
- While a confirmation is on screen, two more global shortcuts exist, and only then: Esc cancels it, and Ctrl+Shift+Enter confirms a restart or shutdown (voice never does). Ctrl+Shift+Enter works only after the prompt has been spoken, and not in the first half second after that. Ctrl+Shift+Space cancels too. If Ctrl+Shift+Enter cannot be registered, the request is cancelled and the tray tooltip says "Confirm key unavailable".
- Click the pill to toggle the "thinking" state.
