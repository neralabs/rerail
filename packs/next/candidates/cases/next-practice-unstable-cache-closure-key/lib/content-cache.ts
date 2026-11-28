import { unstable_cache } from "next/cache";

import { getSession } from "@/lib/auth";

type QueryOptions = { ttl: number; staleOnError: boolean };

async function backedQuery<T>(key: string, fetcher: () => Promise<T>, options: QueryOptions): Promise<T> {
  try {
    return await fetcher();
  } catch (error) {
    if (options.staleOnError) console.warn(`stale ${key}`, error);
    throw error;
  }
}

// A wrapper: its callers pass the computation and build the key from their own params (`posts:${cursor}`). The
// fetcher and options it closes over are theirs, fixed per key.
export async function cachedQuery<T>(key: string, fetcher: () => Promise<T>, options: QueryOptions): Promise<T> {
  const cached = unstable_cache(() => backedQuery<T>(key, fetcher, options), [key], { revalidate: options.ttl });
  return cached();
}

// A wrapper that reads the session itself: the user is its own, not a caller's, and the key does not hold it.
export async function cachedForUser<T>(key: string, fetcher: (userId: string) => Promise<T>): Promise<T> {
  const session = await getSession();
  return unstable_cache(() => fetcher(session.user.id), [key])(); // caught
}

// Not a wrapper: two data parameters, keyed on one.
export async function getTeamPosts(teamId: string, authorId: string) {
  return unstable_cache( // caught
    () => backedQuery(`posts:${teamId}`, async () => ({ teamId, authorId }), { ttl: 60, staleOnError: true }),
    ["team-posts", teamId],
  )();
}
