/** One round's calls: whether each has come, and its release. */
interface Round {
  readonly arrived: ReadonlyArray<PromiseWithResolvers<void>>;
  readonly released: ReadonlyArray<PromiseWithResolvers<void>>;
}

/** Calls held in rounds (`releasedInOrder`), and `drain`, which lets every call held now or later go on at once. */
export interface ReleasedInOrder {
  (round: number, slot: number): Promise<void>;
  drain(): void;
}

const resolvers = (count: number): Array<PromiseWithResolvers<void>> => Array.from({ length: count }, () => Promise.withResolvers<void>());

/**
 * Calls held in rounds, one call from each of `count` callers a round (its `slot`, 0 to `count - 1`): a round's calls
 * wait until all of them have come, then go on one at a time, each a turn of the event loop after the one before, in the
 * order `order(round)` names. A walk that asks for several paths at once then meets their answers in the order the test
 * chooses, whatever the machine's speed.
 */
export function releasedInOrder(count: number, order: (round: number) => readonly number[]): ReleasedInOrder {
  const rounds = new Map<number, Round>();
  let drained = false;
  const roundOf = (round: number): Round => {
    const known = rounds.get(round);
    if (known !== undefined) return known;
    const made: Round = { arrived: resolvers(count), released: resolvers(count) };
    rounds.set(round, made);
    void Promise.all(made.arrived.map(arrived => arrived.promise)).then(async () => {
      for (const slot of order(round)) {
        made.released[slot]?.resolve();
        await new Promise(resolve => setImmediate(resolve));
      }
    });
    return made;
  };
  const held = async (round: number, slot: number): Promise<void> => {
    if (drained) return;
    const { arrived, released } = roundOf(round);
    arrived[slot]?.resolve();
    await released[slot]?.promise;
  };
  return Object.assign(held, {
    drain(): void {
      drained = true;
      for (const { released } of rounds.values()) for (const release of released) release.resolve();
    },
  });
}
