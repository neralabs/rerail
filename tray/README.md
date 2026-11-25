# tray

`rerail-tray`: the system tray icon the daemon starts as a child process while it runs. It is a separate program, not
linked into the engine addon.

The daemon writes the menu's state to the tray's stdin as JSON lines, and the tray writes clicks back on stdout as JSON
lines (`{"action":"open"}`, `{"action":"update"}`, `{"action":"stop"}`). The tray exits when stdin closes, so it never
outlives the daemon. It reads nothing from the Rerail home and makes no network calls.

| Path | What it holds |
|---|---|
| `src/protocol.rs` | the menu state, the action encoder and the stdin reader |
| `src/art.rs` | the status circle and the badge drawn onto the icon |
| `src/desktop.rs` | the macOS menu bar item and the Windows notification-area icon |
| `src/linux.rs` | the Linux icon, a StatusNotifierItem over D-Bus |
| `src/main.rs` | the program's entry point |
| `assets/icon-template.png` | the Nera "n" alone, black on transparent, 44×44: the macOS template icon |
| `assets/icon.png` | the full-colour Nera square, 32×32: the Windows and Linux icon |
