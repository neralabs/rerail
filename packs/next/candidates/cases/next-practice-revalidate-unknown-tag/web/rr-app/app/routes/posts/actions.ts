import { revalidateTag } from "next/cache";

// The same shape in a React Router app that has no next dependency: not this rule's.
export async function publish() {
  revalidateTag("post", "max");
}
