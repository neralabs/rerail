import { spawn } from "node:child_process";
import { once } from "node:events";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { text } from "node:stream/consumers";

/**
 * Makes and removes, in a loop and as fast as it can, 50 folders `d0` to `d49` each holding `m.py`, and 50 files `f0.py`
 * to `f49.py`, under `folder`, until `stop` exists; then prints how many rounds it made. A build or a test run beside a
 * session does the same to the folders a walk lists.
 */
const CHURN = String.raw`
const fs = require("fs"), path = require("path");
const [folder, stop] = process.argv.slice(1);
let round = 0;
while (!fs.existsSync(stop)) {
  round += 1;
  const made = path.join(folder, "d" + (round % 50)), file = path.join(folder, "f" + (round % 50) + ".py");
  try {
    fs.mkdirSync(made, { recursive: true });
    fs.writeFileSync(path.join(made, "m.py"), "x = 1\n");
    fs.writeFileSync(file, "y = 1\n");
    fs.rmSync(made, { recursive: true, force: true });
    fs.rmSync(file, { force: true });
  } catch {}
}
console.log(round);
`;

/** A process churning `folder` (`CHURN`); `stop` ends it and resolves to the rounds it made. */
export function startChurn(folder: string): { stop(): Promise<number> } {
  const stopFile = join(folder, "..", `${Date.now()}-churn.stop`);
  const churn = spawn(process.execPath, ["-e", CHURN, folder, stopFile], { stdio: ["ignore", "pipe", "inherit"], windowsHide: true });
  const rounds = text(churn.stdout);
  const closed = once(churn, "close");
  return {
    async stop() {
      await writeFile(stopFile, "");
      await closed;
      return Number((await rounds).trim());
    },
  };
}
