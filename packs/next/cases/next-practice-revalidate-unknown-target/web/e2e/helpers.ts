import { revalidatePath, revalidateTag } from "next/cache";

export function resetCaches() {
  revalidatePath("/nowhere");
  revalidateTag("fixtures");
}
