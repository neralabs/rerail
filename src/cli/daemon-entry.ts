/**
 * The daemon's process entry (`ENTRY_POINTS.daemon`). It sits in the CLI layer so it can hand the daemon what the
 * layers above it provide, such as set-up's coding agent check, which the daemon layer may not import.
 */
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { runRerailDaemon } from "../daemon/daemon.js";
import { codingAgentHome, codingAgentStates, registrationRoot } from "../setup/coding-agents.js";
import { checkNodeVersion } from "../setup/node-version.js";

// Whatever starts the daemon passes on its own node, which can be an older `node` from PATH once the one set-up
// recorded is gone; the daemon refuses it with set-up's message, which its starter shows from the log.
const node = checkNodeVersion(process.versions.node, "agent");
if (!node.ok) {
  process.stderr.write(node.message);
  process.exit(1);
}

try {
  await runRerailDaemon(process.argv.slice(2), {
    codingAgents: () => codingAgentStates(registrationRoot(defaultRerailHome(), PACKAGE_ROOT), codingAgentHome()),
  });
} catch (error) {
  process.stderr.write(`rerail daemon: ${(error as Error).message}\n`);
  process.exit(1);
}
