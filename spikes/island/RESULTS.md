# Island spike results (2026-11-01)

Setup: Tauri 2, React + TypeScript + Vite, plain CSS animations, release build (`--no-bundle`).

| Check | Result |
|---|---|
| Transparent borderless window | Works: only the pill is drawn, desktop shows around it |
| Top-center positioning | Works |
| Ctrl+Shift+Space toggle with drop-in / slide-up animation | Works |
| Click to switch state (`focusable: false`) | Works |
| Always on top over other windows | Works |
| Tray icon: double-click shows, menu Show/Quit | Works |
| Release exe size | 4.3 MB |
| RAM (host + 6 WebView2 processes) | ~170 MB private, 24 MB host alone |

## Notes
- The "~20 MB" Tauri figure only counts the Rust host; WebView2 dominates memory.
- Candidate optimization: lower the WebView2 memory target level while the island is hidden.
- The island overlaps the browser tab strip; needs a fullscreen/overlap policy in the product.
