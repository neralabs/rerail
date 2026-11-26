"use cache";

import { headers } from "next/headers";
import { prisma } from "@/lib/db";

export async function getMenu() {
  const h = await headers(); // caught
  return prisma.menu.findMany({ where: { host: h.get("host") ?? "" } });
}

export const getFooter = async (locale: string) => {
  return prisma.footer.findFirst({ where: { locale } });
};
