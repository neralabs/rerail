# Falling Facts

A local falling block game. Rerail runs every game rule in [game.rr](game.rr).
The browser sends events as facts and displays the resulting screen facts.

## Play

From the repository root, run:

```sh
npm --prefix tetris start
```

Open <http://127.0.0.1:4317>.
The server listens on the loopback interface. Nothing is published or pushed.
Each server process has one game. Multiple browser tabs share that game.

Use the arrow keys to move, rotate, and soft drop.
Use Space for a hard drop, Z for reverse rotation, P to pause, and R to restart.
The page also has touch controls and a live fact inspector.

## Inspect one round at a time

Select **Manual**, or press **M**, to stop automatic time.
Press **N** to advance 100 ms.
Press **G** to advance one gravity interval at the current level.
Movement and rotation keys still work between time steps.
The rules ignore timer facts in manual mode, including facts from a stale browser tab.
Restart preserves the selected time mode.

Open **View the facts** to inspect the latest round.
Green lines show added facts. Red lines show removed facts. Gray lines show unchanged facts.
Select **Changes only** to hide unchanged facts.
Changed board cells have a light outline, and changed counters have an underline.

Open **Watch the engine** for the optional evaluation panel.
The panel selects manual time so that the game stays still during inspection.
The panel also has buttons to advance game time.

- Use **Play**, **Pause**, and **Step** to inspect recorded rule executions.
- Use the timeline to move through a round without changing the game.
- Use the round selector to inspect one of the last 32 recorded rounds.
- Select a node to inspect its facts and the rule that produced them.
- Drag the graph to pan. Use **+** and **−** to zoom. Use **Fit graph** to restore the view.
- Use the expand button for a larger view.

The slider band marks dependency waves in the recorded native execution order.
A wave ends before a rule reads a fact set produced earlier in that wave.
Click a wave marker to stop just before its final evaluation.
The wave label shows the current wave and the number of completed evaluations within it.

Unchanged facts stay neutral. Facts updated in the current wave have a soft green highlight.
Facts updated in earlier waves have a muted teal highlight.
These states apply to nodes, fact tuples, and screen-grid cells.
The frontier stays visible while playback is paused.
Cache hits do not reset the age of unchanged facts.
At a wave boundary, the next wave starts and completed updates become earlier-wave updates.
Seeking clears transient glows. Returning to the start restores the initial, neutral fact snapshot.

Green packets show accepted additions and removals. Amber packets show unchanged results or cache hits.
Rejected packets retreat a few pixels and briefly thicken the receiver's border.
Accepted updates give the receiving node a bright green glow.
Gray packets show rules with no matching candidates.
Empty results stay in the trace. Select **Animate empty results** to include them in playback.

Graph nodes represent relations (fact sets). The detail pane displays their individual facts.
The corner diamond shows the number of facts. Empty sets have an `∅` sigil.
Nodes list up to four facts directly. Larger sets show a sample.
Screen cells appear as miniature board and preview grids.
Select the screen relation for larger grids and access to the raw tuples.
Edges come from the native parser's rule dependencies, including negative and aggregate reads.
The animation replays recorded execution events at a readable speed. It does not run the game rules in JavaScript.

Each round starts with the previous computed facts visible, including the complete board and preview.
The native cache records each rule's previous result, even when tracing is off.
Playback replaces those results as the new round progresses.
A fact remains visible while another rule still supplies it.
Unchanged facts retain their identity instead of appearing as new facts on each round.

The firing-rule panel displays the original rule source and its file location.
Hover an edge for a source tooltip. Click the edge to pin that rule in the source view underneath.
Select **Follow firing rule** to return to automatic source tracking.
The native lowerer records source lines for each clause, including any generated aggregate helpers.

## Browse the .rr source

Select **Browse .rr source** in the page header to open the full source browser.
The browser includes the complete running game program and the pinned engine's libraries and programs.
The catalog contains 142 `.rr` files at the current engine revision.
Engine reference files have a label that distinguishes them from the running game program.

Use the file filter to find a source file.
Use **Find** to search within that file, or **Line** to move to a line number.
Use the arrow buttons to move between matching lines.
Select **Wrap** to wrap long lines.
The browser highlights relations, variables, directives, types, numbers, strings, comments, and operators.
It reads engine sources from the committed revision recorded by the local build.

The evaluation panel also has a **Full source browser** button.
This button opens the game program at the selected or firing rule's source line.

The local standalone engine adapter caches each rule's result by its exact input fact sets.
The input sets include negative and aggregate dependencies.
If these sets stay equal, the adapter restores the cached result and skips the rule query.
The graph shows this actual short circuit as an amber return.
The native trace also distinguishes duplicate candidates rejected by `INSERT OR IGNORE` from empty results.
Tracing includes observer queries to capture candidates and inserted rows.
Reported round times include this extra work when tracing is enabled.

The cache and observer are local additions to the standalone adapter.
They do not change the Final shape source worktree.
Closing the panel disables trace collection.

The game includes seven shapes, rotation kicks, a landing ghost, a next-piece preview, line clearing, scoring, and levels.
Rotation uses a small kick table. Piece selection uses a seeded generator, without a seven-piece bag.
A blocked descent locks the piece immediately.

## Build

The local native executable is already built in `.cache/`.
If you delete the cache, run:

```sh
npm --prefix tetris run build
```

The build reads commit `f2a67f63e` from the sibling `wolfeel` worktree, which contains the Final rerail shape engine.
It copies committed engine sources into an ignored local directory.
It adds a generic fact-evaluation entry point and a JSON-lines transport.
It does not change tracked files in the source worktree.

To use another source location or revision, set `RERAIL_SOURCE` or `RERAIL_REVISION`.
The source must contain the Final shape logic engine and the `profiling` Cargo profile.
The build needs Git, Cargo, and the engine dependencies.
It reuses the source worktree's Cargo target directory by default.
To use a separate build directory, set `RERAIL_TARGET_DIR` to an absolute path.

## Fact boundary

For example, the driver submits these event facts in separate evaluations:

```text
keypress("ArrowLeft").
timestep(60).
```

The driver also supplies an `entropy(seed)` fact.
The rules use that seed only when a game starts or restarts.
In real-time mode, gravity uses elapsed milliseconds from timestep facts.
In manual mode, the rules derive elapsed time from the step keypress facts.

Each evaluation reads the current state facts and one event fact.
The engine derives `next_*` state relations and `screen_*` relations.
The driver replaces all state facts with the `next_*` outputs before the next evaluation.
An empty output relation deletes its previous state.
The driver has no movement, collision, scoring, or rendering decisions about game state.

| Relation | Purpose |
| --- | --- |
| `active`, `board` | Falling piece and settled cells |
| `stats`, `mode`, `rng`, `clock`, `time_mode` | Score, progress, mode, generator, gravity timer, and time control |
| `screen_cell(area, x, y, color, style)` | One fact per board or preview cell, including empty cells |
| `screen_text(id, value)` | Screen labels, counters, status, and overlays |
| `screen_control(position, keys, label)` | Visible keyboard controls |

Every frame has 200 board cells and 16 preview cells.
CSS defines their appearance. The browser places each cell at the coordinates in its fact.
HTML defines the layout and decorative marks. All game content comes from screen facts.

## Files

| File | Purpose |
| --- | --- |
| `game.rr` | All game rules, state transitions, shape data, and screen projection |
| `facts-plan.rs`, `facts-api.rs` | Generic adapter, exact-input rule cache, and native evaluation observer |
| `facts-main.rs` | Generic native JSON-lines transport |
| `facts-source.rs` | Native rule attribution to original source lines |
| `engine.mjs` | Engine process, fact encoding, and `next_*` state feedback |
| `server.mjs` | Static assets and event-fact HTTP endpoint |
| `browser.mjs` | Keyboard capture, timestep capture, and screen-fact display |
| `evaluation.mjs` | Interactive SVG playback of native rule events |
| `waves.mjs` | Dependency-wave boundaries and frontier-age categories |
| `rr-source.mjs`, `source-browser.mjs` | Syntax highlighting and the full source browser |
| `sources.mjs` | Read-only catalog of the running game and pinned `.rr` source files |

The native adapter contains no game-specific code and no game-specific SQL.
Rerail's existing executor evaluates the Datalog rules through its SQL backend.
The application has no JavaScript or Rust implementation of the game rules.
No additional npm packages are required to play.
All frontend assets work offline.

## Tests

Run the engine-backed tests:

```sh
npm --prefix tetris test
```

These tests cover gravity, walls, rotation kicks, drops, pause, game over, restart, line clearing, and frame invariants.
They include a four-line clear and a sequence of 40 gameplay events.
They also cover manual time, fact differences, trace reconstruction, duplicate rejection, and cache invalidation.
Cache checks include changes to equal-size input sets, negative dependencies, and aggregate dependencies.

If the server is running and the source checkout has Playwright installed, run the browser checks:

```sh
npm --prefix tetris run test:browser
```

Browser checks cover desktop and mobile layouts, keyboard and touch input, manual time, fact colors, graph playback, and offline assets.
The checks observe an actual amber cache bounce, rule stepping, round history, and the expanded graph view.
They also check small rejection recoils, accepted-update glows, corner sigils, screen grids, edge tooltips, and source selection.
Round checks cover the initial fact snapshot, shared facts, and final screen reconstruction.
Source checks cover the full catalog, syntax colors, file selection, search, and line navigation.
Wave checks cover dependency barriers, native evaluation order, clickable slider markers, and persistent frontier colors when seeking.
Screenshots are saved under `.cache/`.
