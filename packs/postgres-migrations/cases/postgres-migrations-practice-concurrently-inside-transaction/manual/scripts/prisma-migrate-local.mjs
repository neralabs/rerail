// A local helper around `prisma migrate dev` for schema work: a JS script is not read as the deploy path
import { spawnSync } from "child_process";
spawnSync("npx prisma migrate dev --create-only", { stdio: "inherit", shell: true });
