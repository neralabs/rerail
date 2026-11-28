"use server";

import bcrypt from "bcrypt";
import { redirect } from "next/navigation";
import { prisma } from "@/lib/db";

export async function signIn(formData: FormData) {
  const email = String(formData.get("email"));
  const user = await prisma.user.findUnique({ where: { email } });
  if (!user || !(await bcrypt.compare(String(formData.get("password")), user.passwordHash))) {
    return { error: "Invalid email or password" };
  }
  redirect("/orders");
}
