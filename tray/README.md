# tray

`rerail-tray`: the system tray icon the daemon starts as a child process while it runs. It is a separate program, not
linked into the engine addon. It is in [internal testing](../docs/internal-testing.md): the daemon starts it only with
`RERAIL_EXPERIMENTAL_TRAY_ICON=1`.

The daemon writes the menu's state to the tray's stdin as JSON lines, and the tray writes clicks back on stdout as JSON
lines: `{"action":"open"}`, and `settings`, `docs`, `repositories`, `update`, `restart` or `stop` in the same shape; a
repository's row sends `{"action":"repository","repository":"<key>"}` with the key the daemon gave the row. The tray
exits when stdin closes, so it never outlives the daemon. It reads nothing from the Rerail home and makes no network
calls; what it shows, repository names included, comes from the daemon.

| Path | What it holds |
|---|---|
| `src/protocol.rs` | the menu state, the action encoder and the stdin reader |
| `src/art.rs` | the status circle, and the "!" badge drawn onto the icon when something needs attention |
| `src/desktop.rs` | the macOS menu bar item and the Windows notification-area icon |
| `src/linux.rs` | the Linux icon, a StatusNotifierItem over D-Bus |
| `src/main.rs` | the program's entry point |
| `assets/icon-template.png` | the Rerail mark alone, black on transparent, 30×44 (12 pt tall in the 22 pt menu bar at 2×, as tall as the system's solid icons): the macOS template icon |
| `assets/icon.png` | the full-colour Rerail square, the blue mark on its light ground, 32×32: the Windows and Linux icon |
