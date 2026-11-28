// Not next/cache's: a helper of this app that happens to share the names.
export function revalidatePath(path: string) {
  console.log("would refresh", path);
}

export function revalidateTag(tag: string) {
  console.log("would refresh", tag);
}

revalidatePath("/nowhere");
revalidateTag("nothing");
