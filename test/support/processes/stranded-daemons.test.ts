import { describe, expect, it } from "vitest";
import { strandedDaemons } from "./stranded-daemons.js";

/**
 * Prints a process list as `ps -A -o pid=,ppid=,command=` does, at least as many bytes as its argument: other
 * processes, then a daemon another process started, then last a daemon this script's parent (the test worker) started.
 */
const PRINT_PROCESS_LIST = `
const lines = [];
for (let bytes = 0, pid = 1000; bytes < Number(process.argv[1]); pid += 1) {
  const line = pid + " 1 /usr/bin/node /srv/app/worker.js --worker " + pid;
  lines.push(line);
  bytes += line.length + 1;
}
lines.push("900 1 node /other/dist/daemon.js --state-file /other/home/daemon.json");
lines.push("901 " + process.ppid + " node /rerail/dist/daemon.js --state-file /rerail/home/daemon.json --dev");
process.stdout.write(lines.join("\\n") + "\\n");
`;

// Windows has no `ps`, and the setup does not look for stranded daemons there.
describe.skipIf(process.platform === "win32")("strandedDaemons", () => {
  it("reads a process list longer than execFileSync's default 1 MB to its end", () => {
    const listBytes = 2 * 1024 * 1024;
    const listProcesses = [process.execPath, "-e", PRINT_PROCESS_LIST, String(listBytes)] as const;

    expect(strandedDaemons(listProcesses)).toEqual([{ pid: 901, home: "/rerail/home" }]);
  });
});
