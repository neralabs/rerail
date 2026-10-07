import { existsSync } from "node:fs";

const candidates = [
  new URL("../../rerail-fixtures/", import.meta.url),
  new URL("../rerail-fixtures/", import.meta.url),
];

export function rerailOraclesDirectory(): URL {
  const directory = candidates.find(candidate => existsSync(candidate));
  if (directory === undefined) {
    throw new Error(`Rerail conformance oracles not found; checked ${candidates.map(candidate => candidate.pathname).join(", ")}`);
  }
  return directory;
}
