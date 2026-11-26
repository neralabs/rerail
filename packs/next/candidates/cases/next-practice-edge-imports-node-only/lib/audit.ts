import { PrismaClient } from "@prisma/client";
import { appendFileSync } from "node:fs";

let client: PrismaClient | undefined;

// Built on first call, from Node.js routes only: loading this module on the edge does not throw.
export function auditDb() {
  client ??= new PrismaClient();
  return client;
}

export function auditToFile(line: string) {
  appendFileSync("/var/log/shop/audit.log", `${line}\n`);
}
