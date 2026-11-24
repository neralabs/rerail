import { unstable_cache } from "next/cache";

// Every read goes through this wrapper, so its tags come from the callers.
export function cached<T>(fn: () => Promise<T>, keys: string[], tags: string[]) {
  return unstable_cache(fn, keys, { tags, revalidate: 300 });
}
