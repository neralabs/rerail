/**
 * A removal Windows refuses for a moment, for a test file that mocks node:fs/promises (`vi.mock`, with `spy: true` or
 * a factory giving `rm` as a `vi.fn`): antivirus, the search indexer or a process that just exited holds a file in
 * the folder, and lets go a moment later.
 */
import type { rm } from "node:fs/promises";
import type { MockedFunction } from "vitest";

/**
 * Makes `remove`, the test file's mocked `rm`, fail the first `times` removals of a path `held` picks with EBUSY, as
 * Windows fails one while another program holds a file in the folder; every other removal, and each after those, is
 * `actual`'s. Returns the paths it refused, one entry per refusal.
 */
export function holdRemovals(
  remove: MockedFunction<typeof rm>,
  actual: typeof rm,
  times: number,
  held: (path: string) => boolean = () => true,
): string[] {
  const refused: string[] = [];
  remove.mockImplementation(async (path, options) => {
    if (refused.length === times || !held(String(path))) return await actual(path, options);
    refused.push(String(path));
    throw Object.assign(new Error(`EBUSY: resource busy or locked, rmdir '${String(path)}'`), { code: "EBUSY", path: String(path) });
  });
  return refused;
}
