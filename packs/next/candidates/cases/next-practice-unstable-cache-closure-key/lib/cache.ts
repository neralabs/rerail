import { unstable_cache } from "next/cache";

// unstable_cache with a JSON round trip, so Dates and Decimals come back as the caller expects.
export function cache<A extends unknown[], T>(fn: (...args: A) => Promise<T>, keys: string[]) {
  const wrap = async (...args: A) => JSON.stringify(await fn(...args));
  const cached = unstable_cache(wrap, keys);
  return async (...args: A): Promise<T> => JSON.parse(await cached(...args));
}
