# tray

`rerail-tray`: the system tray icon the daemon starts as a child process while it runs. It is a separate program, not
linked into the engine addon.

The daemon writes the menu's state to the tray's stdin as JSON lines, and the tray writes clicks back on stdout as JSON
lines (`{"action":"open"}`, `{"action":"update"}`, `{"action":"stop"}`). The tray exits when stdin closes, so it never
outlives the daemon. It reads nothing from the Rerail home and makes no network calls.

| Path | What it holds |
|---|---|
| `src/protocol.rs` | the menu state, the action encoder and the stdin reader |
| `src/main.rs` | the program's entry point |
