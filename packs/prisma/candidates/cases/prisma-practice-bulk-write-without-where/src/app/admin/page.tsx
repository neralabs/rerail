import { revalidatePath } from "next/cache";
import { prisma } from "@/lib/db";

export default async function AdminPage({ searchParams }: { searchParams: Promise<{ kind?: string }> }) {
  const { kind } = await searchParams;

  async function purgeNotifications() {
    "use server";
    await prisma.notification.deleteMany({}); // caught
    revalidatePath("/admin");
  }

  async function purgeKind() {
    "use server";
    await prisma.notification.deleteMany({ where: { kind: kind ?? "digest" } });
    revalidatePath("/admin");
  }

  return (
    <form>
      <button formAction={purgeNotifications}>Clear notifications</button>
      <button formAction={purgeKind}>Clear {kind}</button>
    </form>
  );
}
