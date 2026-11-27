/** The answers awaited now (`AwaitedAnswers`), which the walk of the other worktrees lets go first. */
import { describe, expect, it, vi } from "vitest";
import { AwaitedAnswers } from "../../../../src/engine/code-rule/worktree-walks.js";

describe("the answers awaited", () => {
  it("lets a walk go on only once the last of two answers awaited at once settles", async () => {
    const answers = new AwaitedAnswers();
    expect(answers.none()).toBeUndefined();
    const [first, second] = [Promise.withResolvers<void>(), Promise.withResolvers<void>()];
    const settled = [answers.during(async () => await first.promise), answers.during(async () => await second.promise)];
    let free = false;
    void answers.none()!.then(() => {
      free = true;
    });
    first.resolve();
    await settled[0];
    expect(free).toBe(false);
    second.resolve();
    await settled[1];
    await vi.waitFor(() => expect(free).toBe(true));
    expect(answers.none()).toBeUndefined();
  });
});
