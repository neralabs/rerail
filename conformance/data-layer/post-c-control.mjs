import { writeFileSync } from "node:fs";
import { replay } from "./replay.ts";

const [recording, policies, stem] = process.argv.slice(2);
const start = process.cpuUsage();
const wall = performance.now();
const passed = await replay(recording, stem + "-parity.json", policies);
const cpu = process.cpuUsage(start);
writeFileSync(stem + ".json", JSON.stringify({
  passed, cpuMs: (cpu.user + cpu.system) / 1000,
  userMs: cpu.user / 1000, systemMs: cpu.system / 1000,
  wallMs: performance.now() - wall, maxRSSKiB: process.resourceUsage().maxRSS,
}, null, 2) + "\n");
if (!passed) process.exitCode = 1;
