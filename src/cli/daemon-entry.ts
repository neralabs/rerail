/**
 * The daemon's process entry (`ENTRY_POINTS.daemon`). It sits in the CLI layer so it can hand the daemon what the
 * layers above it provide, such as set-up's coding agent check, which the daemon layer may not import.
 */
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { scrubWrites } from "../engine/secret-scrub.js";
import { runHomeDaemon } from "../daemon/daemon.js";
import { codingAgentHome, codingAgentStates, registrationRoot } from "../setup/coding-agents.js";
import { checkNodeVersion } from "../setup/node-version.js";
import { stampLines } from "./daemon-log.js";

// The daemon's stdout and stderr are `daemon.log`: what Rerail logs there is scrubbed of secrets, then each line starts
// with the time it was written. The wrapper set last runs first, so the time is added to text already scrubbed. The two
// streams write to one file, so they share one count of where a line starts.
stampLines([process.stdout, process.stderr]);
for (const stream of [process.stdout, process.stderr]) scrubWrites(stream);

// Whatever starts the daemon passes on its own node, which can be an older `node` from PATH once the one set-up
// recorded is gone; the daemon refuses it with set-up's message, which its starter shows from the log.
const node = checkNodeVersion(process.versions.node, "agent");
if (!node.ok) {
  process.stderr.write(node.message);
  process.exit(1);
}

try {
  await runHomeDaemon(process.argv.slice(2), {
    codingAgents: () => codingAgentStates(registrationRoot(defaultRerailHome(), PACKAGE_ROOT), codingAgentHome()),
  });
} catch (error) {
  process.stderr.write(`rerail daemon: ${(error as Error).message}\n`);
  process.exit(1);
}
