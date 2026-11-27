// A test process that keeps a privilege switch open until something ends it from outside, for backup-privileges.test.ts:
// `node switch-holder.mjs <path of privilege-switch.ts>` starts the switch, prints its PowerShell's process id once that
// PowerShell is ready, and then waits. Node strips the module's types as it loads it.
import { pathToFileURL } from "node:url";

const { startPrivilegeSwitch } = await import(pathToFileURL(process.argv[2] ?? "").href);
const kept = await startPrivilegeSwitch();
process.stdout.write(`${kept.pid}\n`);
setInterval(() => undefined, 1_000_000);
