/**
 * The daemon's process entry (`ENTRY_POINTS.daemon`). It sits in the CLI layer so it can hand the daemon what the
 * layers above it provide, such as set-up's coding agent check and presentation's status lines, which the daemon layer
 * may not import.
 */
import { join } from "node:path";
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { defaultRerailHome, STATUS_LINES_FOLDER } from "../engine/storage/rerail-home.js";
import { startGitWithoutCopying } from "../engine/git-start.js";
import { loadNativeBinding } from "../engine/native/native.js";
import { scrubWrites } from "../engine/secret-scrub.js";
import { runHomeDaemon } from "../daemon/daemon.js";
import { startDaemonTray } from "../daemon/tray.js";
import type { RerailApiStores } from "../api/operation.js";
import { keepClaudeStatusFiles } from "../presentation/claude-status-files.js";
import { isMainRerailBuild } from "../engine/native/rerail-build.js";
import { codingAgentHome, codingAgentStates, registrationRoot } from "../setup/coding-agents.js";
import { startUpdateChecks, updateCheckApplies } from "../setup/update-check.js";
import { publishedPackReader } from "../setup/published-packs.js";
import { isNewerRelease } from "../setup/updater.js";
import { checkNodeVersion } from "../setup/node-version.js";
import { recordLateCheck } from "../harness/hook-failures.js";
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

// On Linux Node starts a child by copying the whole daemon, which holds its event loop longer the larger it grows; the
// daemon starts its Git through the engine, which copies nothing (`git-start.ts`).
startGitWithoutCopying(loadNativeBinding);

// A home runs one daemon, whatever starts it (`runHomeDaemon`). A TLS key other accounts can read keeps the daemon from
// other machines only: it then serves this one (`serveRerailDaemon`). Claude Code's status line on Windows still starts
// Node, which asks the daemon, so no status lines are kept there.
try {
  await runHomeDaemon(process.argv.slice(2), {
    codingAgents: () => codingAgentStates(registrationRoot(defaultRerailHome(), PACKAGE_ROOT), codingAgentHome()),
    // The hook failure log, where `rerail status` says which events the daemon checked late.
    lateChecked: pass => recordLateCheck(pass),
    // A managed install asks npm once a day whether a newer Rerail is out; a checkout's build never does.
    ...(isMainRerailBuild() && updateCheckApplies(defaultRerailHome()) ? {
      updateChecks: () => startUpdateChecks({ home: defaultRerailHome() }),
    } : {}),
    // A managed install's Add rule packs lists the packs published on npm, read when a person opens it; a checkout's
    // build, which the dashboard's browser tests run, never reads npm.
    ...(isMainRerailBuild() ? { publishedRulePacks: publishedPackReader({ home: defaultRerailHome() }) } : {}),
    ...(process.platform === "win32" ? {} : {
      statusLines: (stores: RerailApiStores) => keepClaudeStatusFiles({ stores, folder: join(defaultRerailHome(), STATUS_LINES_FOLDER) }),
    }),
    // The icon of a main daemon on a desktop, while it runs; the tray decides whether it starts (`trayGate`).
    tray: daemon => startDaemonTray(daemon, { isNewer: isNewerRelease }),
  });
} catch (error) {
  process.stderr.write(`rerail daemon: ${(error as Error).message}\n`);
  process.exit(1);
}
