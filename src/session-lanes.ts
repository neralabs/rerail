type AsyncOperation<T> = () => Promise<T>;

/**
 * Serializes work for one session without coupling unrelated sessions.
 * Settled lanes are removed so completed session ids do not accumulate.
 */
export class SessionLanes {
  readonly #tails = new Map<string, Promise<void>>();

  run<T>(sessionId: string, operation: AsyncOperation<T>): Promise<T> {
    const previous = this.#tails.get(sessionId) ?? Promise.resolve();
    const result = previous.then(operation, operation);
    const tail = result.then(() => undefined, () => undefined);
    this.#tails.set(sessionId, tail);
    void tail.then(() => {
      if (this.#tails.get(sessionId) === tail) this.#tails.delete(sessionId);
    });
    return result;
  }
}
