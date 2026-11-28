"use server";

import { prisma } from "@/lib/db";

// "logIn" inside catalogInfo is no whole word.
export async function updateCatalogInfo(id: string, info: string) { // caught
  await prisma.catalog.update({ where: { id }, data: { info } });
}

// SignUp is a whole camelCase word: public by its name.
export async function handleSignUp(email: string) {
  await prisma.user.create({ data: { email } });
}
