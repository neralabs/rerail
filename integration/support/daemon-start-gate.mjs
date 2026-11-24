// Loaded with `--import` (NODE_OPTIONS) into every Node process a test starts. It acts only in a Rerail daemon process
// (daemon-entry.js) when RERAIL_TEST_DAEMON_START_GATE names a folder: it notes the daemon's pid there, and holds the
// daemon's start right after the daemon first writes daemon.json in its home, until the test writes `go` in that folder.
// The hold blocks the daemon's one thread, as synchronous start work does, so the daemon is recorded and does not
// answer: a daemon still coming up, for as long as the test says, with no sleep and no load on the machine.
import { existsSync, writeFileSync } from "node:fs";
import fsPromises from "node:fs/promises";
import { syncBuiltinESMExports } from "node:module";
import { basename, join, resolve } from "node:path";

const gate = process.env.RERAIL_TEST_DAEMON_START_GATE;
const home = process.env.RERAIL_HOME;
if (gate !== undefined && home !== undefined && basename(process.argv[1] ?? "") === "daemon-entry.js") {
  writeFileSync(join(gate, `daemon-${process.pid}`), "");
  const record = resolve(home, "daemon.json");
  const rename = fsPromises.rename;
  let held = false;
  fsPromises.rename = async (from, to) => {
    await rename(from, to);
    if (held || resolve(String(to)) !== record) return;
    held = true;
    writeFileSync(join(gate, `held-${process.pid}`), "");
    const word = new Int32Array(new SharedArrayBuffer(4));
    while (!existsSync(join(gate, "go"))) Atomics.wait(word, 0, 0, 20);
  };
  // The daemon's modules import `rename` by name, which reads the builtin's export as this sync leaves it.
  syncBuiltinESMExports();
}
