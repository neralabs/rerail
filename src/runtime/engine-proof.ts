import { createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import { close, open, write } from "node:fs";
import { request, type RequestOptions } from "node:http";
import { Socket } from "node:net";
import { Duplex } from "node:stream";
import { setTimeout as sleep } from "node:timers/promises";
import { promisify } from "node:util";
import { isNamedPipe } from "./engine-pipe.js";

/**
 * How a process asks whether the engine on a socket or pipe is this Rerail home's without presenting the home's access
 * capability: it sends a random challenge, and only an engine that holds the capability can answer with
 * {@link engineProof}. On Windows any local user can create the engine pipe's name first, so a probe that sent the
 * capability itself would hand it to whoever holds the name.
 */
export const ENGINE_PROOF_PATH = "/proof";
/** The header a challenge travels in: 64 lowercase hex characters, 32 random bytes. */
export const ENGINE_CHALLENGE_HEADER = "x-rerail-challenge";
const CHALLENGE = /^[0-9a-f]{64}$/u;
/** A proof as {@link engineProof} writes one: an HMAC-SHA-256 in 64 lowercase hex characters. */
const PROOF = CHALLENGE;
/** Longest answer read: the proof is a JSON object holding 64 hex characters. */
const MAX_ANSWER_BYTES = 1_024;
/** How often a proof tries again for a free instance of a Windows pipe whose instances are all taken. */
const PIPE_RETRY_MS = 50;
const openFile = promisify(open);

/** Whether `value` is a challenge as {@link engineHoldsCapability} sends one. */
export function isEngineChallenge(value: unknown): value is string {
  return typeof value === "string" && CHALLENGE.test(value);
}

/**
 * What an engine holding `accessToken` answers `challenge` with: an HMAC keyed by the capability, which tells whoever
 * reads it nothing about the capability. The label keeps it from matching an HMAC made for anything else.
 */
export function engineProof(accessToken: string, challenge: string): string {
  return createHmac("sha256", accessToken).update(`rerail-engine-proof\n${challenge}`).digest("hex");
}

/**
 * Whether `answer` holds `expected` as its proof. Anything that is not a proof as {@link engineProof} writes one (another
 * type, another length, a character outside lowercase hex) is refused before the comparison, so a holder can neither make
 * `timingSafeEqual` throw with an answer of the proof's character count but more bytes, nor pass; and a comparison that
 * throws all the same is a refusal.
 */
function provesChallenge(answer: Buffer, expected: string): boolean {
  try {
    const proof: unknown = (JSON.parse(answer.toString("utf8")) as { proof?: unknown } | null)?.proof;
    return typeof proof === "string" && PROOF.test(proof) && timingSafeEqual(Buffer.from(proof, "latin1"), Buffer.from(expected, "latin1"));
  } catch {
    return false;
  }
}

/**
 * What the listener on a socket or pipe made of a challenge ({@link engineCapabilityProof}): it answered with the proof
 * (`proved`); it refused, answered anything else or hung up, or nothing listens (`refused`); or no whole answer came
 * within the timeout, counted from the ask (`silent`). An engine whose one event loop is inside an audit or a long
 * garbage collection is silent as well as a holder that never answers, so silence proves neither that the engine is
 * this home's nor that it is not.
 */
export type EngineProofAnswer = "proved" | "refused" | "silent";

/**
 * The Windows named pipe `path` opened as a file, its descriptor: an open that finds every instance of the pipe taken
 * fails at once with `EBUSY`, and is tried again every {@link PIPE_RETRY_MS} until `deadline` (epoch ms), when the
 * `EBUSY` is thrown. Node's own connect waits up to 30 s for a free instance, in a thread that closing the socket does
 * not stop, and that thread keeps the process alive after the proof has answered.
 */
async function openPipe(path: string, deadline: number): Promise<number> {
  for (;;) {
    try {
      return await openFile(path, "r+");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EBUSY" || Date.now() >= deadline) throw error;
    }
    await sleep(PIPE_RETRY_MS);
  }
}

/**
 * The stream a proof's request runs over on a pipe {@link openPipe} opened. A pipe opened as a file is synchronous: a
 * read waiting for data holds it, and a write waits behind that read, so the request would never leave. The request is
 * written with `fs.write` first, and only then does a socket on the descriptor start reading the answer. Closing that
 * socket stops its waiting read, so nothing keeps the process alive.
 */
class PipeExchange extends Duplex {
  readonly #fd: number;
  #socket: Socket | undefined;

  constructor(fd: number) {
    super();
    this.#fd = fd;
  }

  override _write(chunk: Buffer, _encoding: BufferEncoding, callback: (error?: Error | null) => void): void {
    write(this.#fd, chunk, (error) => {
      if (error === null) this.#startReading();
      callback(error);
    });
  }

  override _read(): void {
    this.#socket?.resume();
  }

  override _destroy(error: Error | null, callback: (error?: Error | null) => void): void {
    if (this.#socket === undefined) {
      close(this.#fd, () => callback(error));
      return;
    }
    this.#socket.destroy();
    callback(error);
  }

  #startReading(): void {
    // A proof ended while its request was being written has closed the descriptor, which the OS may since have reused.
    if (this.#socket !== undefined || this.destroyed) return;
    const socket = new Socket({ fd: this.#fd, readable: true, writable: false });
    socket.on("data", (chunk: Buffer) => {
      if (!this.push(chunk)) socket.pause();
    });
    socket.on("end", () => this.push(null));
    socket.on("error", error => this.destroy(error));
    this.#socket = socket;
  }
}

/**
 * How a proof's request reaches the listener on `socketPath`: a Windows pipe opened by {@link openPipe} before the
 * request and written then read through {@link PipeExchange}, a Unix socket by its path. Only Windows has named pipes:
 * on macOS and Linux a path spelled like one is a Unix socket's. A pipe that is not opened by `deadline` is the verdict:
 * every instance still taken is no answer (`silent`); a pipe that is gone, or that this user may not open, is `refused`.
 */
async function proofRoute(socketPath: string, deadline: number): Promise<RequestOptions | EngineProofAnswer> {
  if (process.platform !== "win32" || !isNamedPipe(socketPath)) return { socketPath, agent: false };
  try {
    const exchange = new PipeExchange(await openPipe(socketPath, deadline));
    return { createConnection: () => exchange };
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EBUSY" ? "silent" : "refused";
  }
}

/**
 * Asks the process listening on `socketPath` (a Unix socket or a Windows named pipe) to prove it holds `accessToken`: it
 * answers a fresh challenge with {@link engineProof}. Only the challenge is sent; the capability never leaves this
 * process. The answer comes within `timeoutMs` of the ask, whatever the listener does, and nothing of the ask outlives
 * it: the connect is bounded ({@link proofRoute}), and the limit is a timer of its own, since a request's timeout
 * starts only once its socket has connected and restarts at each byte read.
 */
export async function engineCapabilityProof(
  socketPath: string,
  accessToken: string,
  timeoutMs = 2_000,
  challenge: string = randomBytes(32).toString("hex"),
): Promise<EngineProofAnswer> {
  const deadline = Date.now() + timeoutMs;
  const route = await proofRoute(socketPath, deadline);
  if (typeof route === "string") return route;
  const expected = engineProof(accessToken, challenge);
  const { promise, resolve } = Promise.withResolvers<EngineProofAnswer>();
  let answered = false;
  const outgoing = request({
    ...route,
    method: "POST",
    path: ENGINE_PROOF_PATH,
    headers: { [ENGINE_CHALLENGE_HEADER]: challenge, "content-length": 0 },
  }, (response) => {
    answered = true;
    const chunks: Buffer[] = [];
    let size = 0;
    response.on("data", (chunk: Buffer) => {
      size += chunk.length;
      if (size > MAX_ANSWER_BYTES) outgoing.destroy(new Error("Engine proof too long"));
      else chunks.push(chunk);
    });
    response.on("end", () => resolve(response.statusCode === 200 && provesChallenge(Buffer.concat(chunks), expected) ? "proved" : "refused"));
    // The first verdict wins: a response cut short ends here without `end`.
    response.on("close", () => resolve("refused"));
  });
  // The silence is the verdict, before the destroy below reports it as an error.
  const limit = setTimeout(() => {
    resolve("silent");
    outgoing.destroy(new Error("Engine proof timed out"));
  }, Math.max(0, deadline - Date.now()));
  outgoing.on("error", () => resolve("refused"));
  // Node can close the request before the response's `end`; only a request that never got a response is a refusal here.
  outgoing.on("close", () => {
    if (!answered) resolve("refused");
  });
  outgoing.end();
  try {
    return await promise;
  } finally {
    clearTimeout(limit);
  }
}

/**
 * Whether the process listening on `socketPath` holds `accessToken` ({@link engineCapabilityProof}). A listener that
 * refuses, answers anything else, hangs up or stays silent for `timeoutMs` does not hold it.
 */
export async function engineHoldsCapability(
  socketPath: string,
  accessToken: string,
  timeoutMs = 2_000,
  challenge: string = randomBytes(32).toString("hex"),
): Promise<boolean> {
  return await engineCapabilityProof(socketPath, accessToken, timeoutMs, challenge) === "proved";
}
