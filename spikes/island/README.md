# Niva island spike

Throwaway spike: Tauri 2 floating "island" window (Dynamic-Island style) for the Niva assistant.

## Run

```
npm install
npm run tauri dev
```

Release exe without installer: `npm run tauri build -- --no-bundle` -> `src-tauri/target/release/island.exe`.

## Use

- Ctrl+Shift+Space toggles the island (starts hidden). Tray icon: "Show Niva" / "Quit"; double-click shows it.
- Click the pill to toggle the "thinking" state.

## Validates

- Frameless transparent always-on-top window, no taskbar entry, no shadow, top-center on the primary monitor.
- Global shortcut registered in Rust, show/hide driven by events + CSS animations (hide delayed ~250 ms).
- Non-focusable window (`focusable: false`) so showing it does not steal focus.
- Tray menu and double-click, state-driven CSS animation.
