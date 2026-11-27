"use server";

import { prisma } from "@/lib/db";
import { createClient } from "@/lib/supabase/server";

// The auth client's update fails without the caller's session, and the write keys on the user it returns.
export async function changeEmail(email: string) {
  const supabase = await createClient();
  const { data, error } = await supabase.auth.updateUser({ email });
  if (error) throw error;
  const userId = data?.user?.id;
  await prisma.user.update({ where: { id: userId }, data: { email } });
}

// The auth client is called, but the write keys on the id the client sends.
export async function changeEmailFor(userId: string, email: string) { // caught
  const supabase = await createClient();
  await supabase.auth.updateUser({ email });
  await prisma.user.update({ where: { id: userId }, data: { email } });
}
