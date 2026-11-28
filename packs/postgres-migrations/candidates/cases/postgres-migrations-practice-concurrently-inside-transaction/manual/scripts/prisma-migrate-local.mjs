// A local helper for schema work: it writes a new migration from schema.prisma and applies none
import { spawnSync } from "child_process";
spawnSync("npx prisma migrate dev --create-only", { stdio: "inherit", shell: true });
