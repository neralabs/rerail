import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

/** How many delivery entries of `content` the session ledgers under `storageRoot` hold. */
export async function deliveryEntries(storageRoot: string, content: string): Promise<number> {
  const names = await readdir(storageRoot, { recursive: true });
  let count = 0;
  for (const name of names.filter(entry => entry.endsWith("events.jsonl"))) {
    const lines = (await readFile(join(storageRoot, name), "utf8")).split("\n");
    count += lines.filter(line => line.includes('"type":"delivery"') && line.includes(JSON.stringify(content).slice(1, -1))).length;
  }
  return count;
}
