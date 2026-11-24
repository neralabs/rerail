import { auth } from "@/auth";

export async function requireUser() {
  const session = await auth();
  if (!session?.user) throw new Error("Unauthorized");
  return session.user;
}

// A helper whose name says nothing, but which reads the session.
export async function currentOwnerId() {
  const user = await requireUser();
  return user.id;
}
