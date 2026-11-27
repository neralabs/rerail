import { redirect } from "next/navigation";

// The same shape in a React Router app that has no next dependency: not this rule's.
export async function publish(id: string) {
  try {
    redirect(`/posts/${id}`);
  } catch (e) {
    return { error: String(e) };
  }
}
