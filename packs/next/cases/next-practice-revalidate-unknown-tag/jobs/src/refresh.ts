import { revalidatePath, revalidateTag } from "next/cache";

export function teamsChanged(id: string) {
  revalidateTag("tems", "max"); // caught
  revalidateTag(`member:${id}`, "max"); // caught
  revalidateTag("teams", "max");
  revalidateTag(`team:${id}`, "max");
  revalidatePath("/settings/teams");
}
