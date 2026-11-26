// Run at release: applies the pending migrations with Prisma's migrator
import { execSync } from "child_process";
execSync("npx prisma migrate deploy", { stdio: "inherit" });
