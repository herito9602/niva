# Niva app

Tauri 2 desktop app (React + TypeScript + Vite) with a floating "island" window for the Niva assistant.

## Run

Run cargo and Tauri from PowerShell, never Git Bash (Git's `link` shadows MSVC `link.exe`).

```
npm install
npm run tauri dev
```

Tests: `npm test` (vitest) here, `cargo test` at the workspace root.

## Use

- Ctrl+Shift+Space toggles the island (starts hidden). Tray icon: "Show Niva" / "Quit"; double-click shows it.
- Click the pill to toggle the "thinking" state.
