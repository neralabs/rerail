import { beforeEach, it } from "vitest";
import { prisma } from "@/lib/db";

beforeEach(async () => {
  await prisma.session.deleteMany();
});

it("runs", () => {});
