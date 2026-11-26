import { revalidatePath } from "next/cache";

test("refreshes", () => {
  revalidatePath("/nowhere");
});
