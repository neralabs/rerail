import { prisma } from "@/lib/db";
import { auth } from "@/auth";
import { archiveProject } from "@/server/projects";
import { deleteNoteRow, NoteList } from "./note-list";

export default async function NotesPage() {
  const session = await auth();
  if (!session) return null;

  async function addNote(formData: FormData) { // caught
    "use server";
    await prisma.note.create({ data: { body: String(formData.get("body")) } });
  }

  async function removeNote(id: string) { // caught
    "use server";
    await deleteNoteRow(id);
  }

  async function archiveFromNotes(id: string) { // cross-lane: missed until the engine joins .ts and .tsx
    "use server";
    await archiveProject(id);
  }

  const pinNote = async (id: string) => {
    "use server";
    const again = await auth();
    if (!again) throw new Error("Unauthorized");
    await prisma.note.update({ where: { id }, data: { pinned: true } });
  };

  return (
    <form action={addNote}>
      <NoteList ids={[]} />
      <button formAction={pinNote.bind(null, "1")}>Pin</button>
      <button formAction={removeNote.bind(null, "1")}>Remove</button>
      <button formAction={archiveFromNotes.bind(null, "1")}>Archive</button>
    </form>
  );
}
