import { revalidatePath } from "next/cache";
import { prisma } from "@/lib/db";
import { auth } from "@/lib/auth";
import { listForOwner, touchProject } from "@/lib/projects";

export default async function ProjectsPage() {
  const session = await auth();
  const projects = await listForOwner(session.user.id);

  async function archive(id: string) {
    "use server";
    await prisma.project.updateMany({ where: { id }, data: { archived: true } }); // caught
    revalidatePath("/projects");
  }

  async function touch(id: string) {
    "use server";
    await touchProject(id); // cross-lane: missed until the engine joins .ts and .tsx
  }

  async function rename(id: string, formData: FormData) {
    "use server";
    if (typeof id !== "string") throw new Error("invalid id");
    await prisma.project.updateMany({ where: { id }, data: { name: String(formData.get("name")) } });
    revalidatePath("/projects");
  }

  // A plain server-side helper: its argument comes from the page, not from a client.
  async function countOpen(ownerId: string) {
    return prisma.project.count({ where: { ownerId, archived: false } });
  }
  const open = await countOpen(session.user.id);

  return (
    <ul data-open={open}>
      {projects.map((p) => (
        <li key={p.id}>
          <form action={archive.bind(null, p.id)}>
            <button>Archive</button>
          </form>
          <form action={rename.bind(null, p.id)}>
            <input name="name" defaultValue={p.name} />
          </form>
        </li>
      ))}
    </ul>
  );
}
