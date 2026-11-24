import { prisma } from "@/lib/db";

export default function AdminPage() {
  async function purge(formData: FormData) {
    "use server";
    const ids = formData.getAll("id").map(String);
    await prisma.$transaction(async (tx) => {
      ids.forEach((id) => tx.session.deleteMany({ where: { userId: id } })); // caught
      await tx.user.deleteMany({ where: { id: { in: ids } } });
    });
  }
  return (
    <form action={purge}>
      <button type="submit">Purge</button>
    </form>
  );
}
