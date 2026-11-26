import { revalidateTag } from "next/cache";

test("refreshes", () => {
  revalidateTag("nothing-tags-this", "max");
});
