import { prisma } from "@/lib/db";

export async function deleteNoteRow(id: string) {
  await prisma.note.delete({ where: { id } });
}

export function NoteList({ ids }: { ids: string[] }) {
  return <ul>{ids.map((id) => <li key={id}>{id}</li>)}</ul>;
}
