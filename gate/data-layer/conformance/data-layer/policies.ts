/** One rule document per id: shipped ports live in rules/, unfinished ports in candidates/rules/. */
import { existsSync } from "node:fs";
import { basename, dirname, resolve } from "node:path";

export function policyPath(directory: string, id: string): string {
  const candidate = resolve(directory, `${id}.json`);
  if (existsSync(candidate)) return candidate;
  const root = resolve(directory);
  return basename(root) === "rules" && basename(dirname(root)) === "candidates"
    ? resolve(root, "../../rules", `${id}.json`) : candidate;
}
