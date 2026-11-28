import { unstable_cache } from "next/cache";

// A wrapper that forwards its arguments: `opts.fn` is only called, with the cached function's own arguments, which
// Next keys the entry on (`(...opts.args)`).
export async function withCache<T extends (...args: string[]) => Promise<unknown>>(opts: {
  fn: T;
  args: Parameters<T>;
  keys: string[];
  tags: string[];
}) {
  const result = await unstable_cache(
    async (...args) => {
      const value = await opts.fn(...args);
      return value;
    },
    [...opts.keys],
    { tags: opts.tags },
  )(...opts.args);
  return result as Awaited<ReturnType<T>>;
}

// The same wrapper passing a value of its own beside the arguments: the tenant is in neither the key nor the call.
export async function withTenantCache<T extends (...args: string[]) => Promise<unknown>>(opts: {
  fn: T;
  tenantId: string;
  args: Parameters<T>;
  keys: string[];
}) {
  return unstable_cache(async (...args) => opts.fn(opts.tenantId, ...args), [...opts.keys])(...opts.args); // caught
}
