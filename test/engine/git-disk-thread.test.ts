/**
 * The handshake of the thread a blocking look reads the disk in (`askedInThread`): this thread asks a question, and
 * sleeps until the answer comes. Here the answering thread is played in this one, at the moments a thread may act
 * between this thread's steps: the Worker git.ts starts is replaced by one that runs nothing, and this thread's reads of
 * the port (`receiveMessageOnPort`) and waits (`Atomics.wait`) are where it acts. A file of its own: it mocks the module
 * every other look's thread comes from.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("node:worker_threads", { spy: true });

import { type MessagePort, receiveMessageOnPort, Worker } from "node:worker_threads";

import { askedInThread } from "../../src/engine/git.js";

const actual = await vi.importActual<typeof import("node:worker_threads")>("node:worker_threads");

/** How often a played thread lets this thread's wait end at once before it answers: a spin of that many turns. */
const SPIN_LIMIT = 1_000;

/**
 * The answering thread, played as git.ts's own acts: it posts each answer, then counts it in `answered` and wakes this
 * thread. It answers the first question as soon as this thread reads the port after asking it, so that answer is there
 * before this thread waits, and its count lands late, at this thread's next step. Every later question it answers only
 * while this thread sleeps, as a folder off the network answers, or once this thread's waits have ended at once
 * `SPIN_LIMIT` times in a row. `exists` answers whether a path is in `files`.
 */
class PlayedThread {
  readonly #answered: Int32Array;
  readonly #port: MessagePort;
  readonly #files: ReadonlySet<string>;
  readonly #waiting: { path: string }[] = [];
  #answers = 0;
  #lateCount = false;
  /** How many of this thread's waits ended at once, with no answer to read: turns of a spin. */
  spins = 0;

  constructor(workerData: { answered: Int32Array; port: MessagePort }, files: ReadonlySet<string>) {
    this.#answered = workerData.answered;
    this.#port = workerData.port;
    this.#files = files;
  }

  /** Whether `port` is this thread's own end of the channel, which this thread does not read. */
  owns(port: MessagePort): boolean {
    return port === this.#port;
  }

  /** As this thread reads its port: a late count lands; the first question is answered at once, its count late. */
  atRead(): void {
    this.#countLate();
    this.#take();
    if (this.#answers === 0 && this.#waiting.length > 0) this.#answer({ late: true });
  }

  /** As this thread waits while `answered[0]` is `value`: ends at once when it is not, else sleeps until counted. */
  atWait(value: number): "ok" | "not-equal" {
    this.#countLate();
    this.#take();
    if (Atomics.load(this.#answered, 0) !== value) {
      this.spins += 1;
      if (this.spins % SPIN_LIMIT === 0 && this.#waiting.length > 0) this.#answer({ late: false });
      return "not-equal";
    }
    if (this.#waiting.length === 0) throw new Error("this thread sleeps with no question asked: it would sleep to its time limit");
    this.#answer({ late: false });
    return "ok";
  }

  #take(): void {
    for (let received = actual.receiveMessageOnPort(this.#port); received !== undefined; received = actual.receiveMessageOnPort(this.#port)) {
      this.#waiting.push(received.message as { path: string });
    }
  }

  #answer({ late }: { late: boolean }): void {
    const question = this.#waiting.shift()!;
    this.#port.postMessage(this.#files.has(question.path));
    this.#answers += 1;
    this.#lateCount = true;
    if (!late) this.#countLate();
  }

  #countLate(): void {
    if (!this.#lateCount) return;
    this.#lateCount = false;
    Atomics.add(this.#answered, 0, 1);
    Atomics.notify(this.#answered, 0);
  }
}

/** The thread the next look starts, played by a {@link PlayedThread} over `files`. */
function playThread(files: ReadonlySet<string>): { readonly thread: () => PlayedThread } {
  let played: PlayedThread | undefined;
  vi.mocked(Worker).mockImplementationOnce(class {
    constructor(_code: string, options: { workerData: { answered: Int32Array; port: MessagePort } }) {
      played = new PlayedThread(options.workerData, files);
    }

    unref(): void {}

    async terminate(): Promise<number> {
      return await Promise.resolve(0);
    }
  } as unknown as typeof Worker);
  const thread = (): PlayedThread => {
    if (played === undefined) throw new Error("no look started a thread");
    return played;
  };
  vi.mocked(receiveMessageOnPort).mockImplementation(port => {
    if (played !== undefined && !played.owns(port)) played.atRead();
    return actual.receiveMessageOnPort(port);
  });
  const wait = Atomics.wait.bind(Atomics);
  vi.spyOn(Atomics, "wait").mockImplementation((array, index, value, timeout) => {
    if (played === undefined || typeof value !== "number") return wait(array, index, value, timeout);
    return played.atWait(value);
  });
  return { thread };
}

describe("the thread a blocking look reads the disk in", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("sleeps until each answer comes, when the count of the answer before it lands after the next question", () => {
    const { thread } = playThread(new Set(["/opt/git/bin/git"]));
    const answers = askedInThread(10_000, disk => [disk.exists("/opt/git/bin/git"), disk.exists("//share/off/the/network/git")]);
    // Before the fix the second wait ended at once at every turn: the late count of the first answer had set the flag
    // the waiting thread reset before asking, and it spun until the folder off the network answered.
    expect([answers, thread().spins]).toEqual([[true, false], 0]);
  });
});
