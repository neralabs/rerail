import { spawn } from "node:child_process";
import { once } from "node:events";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";

/**
 * Makes and removes, in a loop and as fast as it can, 50 folders `d0` to `d49` each holding `m.py`, and 50 files `f0.py`
 * to `f49.py`, under `folder`, until `stop` exists, writing the count of rounds made after each. A build or a test run
 * beside a session does the same to the folders a walk lists. It writes with `writeSync`: its loop never yields, so a
 * write left to the event loop would never go out.
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
    fs.writeSync(1, round + "\n");
  } catch {}
}
`;

/** A process churning a folder (`startChurn`). */
export interface Churn {
  /** Resolves once the process has made `rounds` more rounds than it had now; rejects if it ends first. */
  after(rounds: number): Promise<void>;
  /** Ends the process and resolves to the rounds it made. */
  stop(): Promise<number>;
}

/** A process churning `folder` (`CHURN`). */
export function startChurn(folder: string): Churn {
  const stopFile = join(folder, "..", `${Date.now()}-churn.stop`);
  const churn = spawn(process.execPath, ["-e", CHURN, folder, stopFile], { stdio: ["ignore", "pipe", "inherit"], windowsHide: true });
  let made = 0;
  let partial = "";
  const waiting: Array<{ readonly rounds: number; readonly reached: PromiseWithResolvers<void> }> = [];
  churn.stdout.setEncoding("utf8");
  churn.stdout.on("data", (chunk: string) => {
    const lines = (partial + chunk).split("\n");
    partial = lines.pop() ?? "";
    if (lines.length > 0) made = Number(lines.at(-1));
    for (const wait of waiting.filter(entry => entry.rounds <= made)) {
      waiting.splice(waiting.indexOf(wait), 1);
      wait.reached.resolve();
    }
  });
  const closed = once(churn, "close").then(() => {
    for (const wait of waiting.splice(0)) wait.reached.reject(new Error(`the churn process ended after ${made} rounds`));
  });
  return {
    async after(rounds) {
      const wait = { rounds: made + rounds, reached: Promise.withResolvers<void>() };
      waiting.push(wait);
      await wait.reached.promise;
    },
    async stop() {
      await writeFile(stopFile, "");
      await closed;
      return made;
    },
  };
}
