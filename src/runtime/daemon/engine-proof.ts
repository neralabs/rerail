import { createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import { once } from "node:events";
import { closeSync, openSync } from "node:fs";
import { request, type RequestOptions } from "node:http";
import { connect, type Socket } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import { isNamedPipe, pipeNameOf } from "./engine-pipe.js";

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
/**
 * How long a proof waits before it tries again a Windows pipe whose instances are all taken. A timer there fires on a
 * tick of the system clock, every 15.6 ms, and this wait, shorter than a tick and longer than a try, ends on the next
 * one: an instance a busy engine frees, or a pipe that goes, is found within a tick. A wait of 1 ms would end there
 * too, but on a machine whose clock ticks every millisecond it would make a thousand tries a second.
 */
const PIPE_RETRY_MS = 10;
/**
 * How long a proof that waits for a free instance of a held Windows pipe waits for one (`waitForInstance`): as long as
 * Node's own connect waits for one, as main's proof did. A busy engine of this home that frees one in that time proves
 * itself, so the engine start starts no second daemon beside it, and a hook's event reaches it.
 */
const INSTANCE_WAIT_MS = 30_000;

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
 * (`proved`); it answered that it has no {@link ENGINE_PROOF_PATH} (`no-proof`, a 404); it refused otherwise, answered
 * anything else or hung up, or nothing listens (`refused`); or no whole answer came within the timeout, counted from the
 * ask (`silent`). An engine of Rerail 0.1.4 or earlier has no proof and answers 404 to every path but its events, as any
 * listener without that path does, and an engine whose one event loop is inside an audit or a long garbage collection
 * is silent as well as a holder that never answers: neither answer proves that the engine is this home's, nor that it is
 * not.
 */
export type EngineProofAnswer = "proved" | "no-proof" | "refused" | "silent";

/** What an answer of `status` with the body `answer` makes of a challenge whose proof is `expected`. */
function proofAnswer(status: number | undefined, answer: Buffer, expected: string): EngineProofAnswer {
  if (status === 404) return "no-proof";
  return status === 200 && provesChallenge(answer, expected) ? "proved" : "refused";
}

/**
 * The Windows named pipe `path` spelled `\\?\pipe\<name>`, which `CreateFile` opens as the same pipe. Node's connect
 * opens it for overlapped I/O, so closing the socket cancels a read or a write in flight, whatever the holder does: a
 * pipe opened as a file is synchronous, and a write that its holder never reads cannot be stopped. When every instance
 * is taken, Node waits for a free one with `WaitNamedPipe`, up to 30 s in a thread that closing the socket does not
 * stop and that keeps the process alive; `WaitNamedPipe` takes only `\\.\pipe\` names, so for this spelling that
 * wait fails at once, with `ENOENT`.
 */
function pipeConnectPath(path: string): string {
  return String.raw`\\?\pipe` + "\\" + pipeNameOf(path);
}

/**
 * Whether a connect to the pipe `path` failed with `error` only because every instance was taken. The connect fails with
 * `ENOENT` both then and when the pipe is gone. A plain open of the pipe tells the two apart in one call (`EBUSY` while
 * every instance is taken), where a listing of `\\.\pipe\` would read every pipe on the machine at each try. An open
 * that gets an instance, freed since the connect, closes it at once, and the proof tries again. An open that fails
 * otherwise counts as busy, so a pipe the open cannot tell about is never read as gone.
 */
function pipeBusy(error: unknown, path: string): boolean {
  if ((error as NodeJS.ErrnoException).code !== "ENOENT") return false;
  try {
    closeSync(openSync(path, "r+"));
    return true;
  } catch (opened) {
    return (opened as NodeJS.ErrnoException).code !== "ENOENT";
  }
}

/** Whether `socket` connected; the error it failed with when it did not. */
async function connected(socket: Socket): Promise<true | { error: unknown }> {
  try {
    await once(socket, "connect");
    return true;
  } catch (error) {
    return { error };
  }
}

/** How a proof waits `ms` milliseconds before it tries a busy Windows pipe again. */
type PipeWait = (ms: number) => Promise<void>;

/**
 * A socket connected to the Windows named pipe `path` ({@link pipeConnectPath}), or the verdict when there is none by
 * `deadline` (epoch ms). A pipe whose instances are all taken is tried again after `wait` of {@link PIPE_RETRY_MS},
 * since a busy engine frees one as soon as its event loop runs again, and is no answer (`silent`) while it stays busy;
 * a pipe that is gone, or that this user may not open, is `refused`.
 */
async function connectPipe(path: string, deadline: number, wait: PipeWait): Promise<Socket | EngineProofAnswer> {
  for (;;) {
    const socket = connect(pipeConnectPath(path));
    const outcome = await connected(socket);
    if (outcome === true) return socket;
    socket.destroy();
    if (!pipeBusy(outcome.error, path)) return "refused";
    if (Date.now() >= deadline) return "silent";
    await wait(PIPE_RETRY_MS);
  }
}

/**
 * How a proof's request reaches the listener on `socketPath`: a Windows pipe connected by {@link connectPipe} by
 * `deadline` (epoch ms) before the request, a Unix socket by its path. Only Windows has named pipes: on macOS and Linux
 * a path spelled like one is a Unix socket's. A pipe with no connection is the verdict.
 */
async function proofRoute(socketPath: string, deadline: number, wait: PipeWait = sleep): Promise<RequestOptions | EngineProofAnswer> {
  if (process.platform !== "win32" || !isNamedPipe(socketPath)) return { socketPath, agent: false };
  const socket = await connectPipe(socketPath, deadline, wait);
  return typeof socket === "string" ? socket : { createConnection: () => socket };
}

/**
 * What the listener `route` reaches answers `challenge` with, by `answerBy` (epoch ms). The limit is a timer of its own,
 * since a request's timeout starts only once its socket has connected and restarts at each byte read. Once there is a
 * verdict the request is destroyed with its socket, so a listener that keeps the connection open holds nothing here.
 * The Host header is the one Node writes for a Unix socket, on a pipe too: a request with a connection of its own and
 * no agent would carry `localhost:80`.
 */
async function askForProof(route: RequestOptions, accessToken: string, challenge: string, answerBy: number): Promise<EngineProofAnswer> {
  const expected = engineProof(accessToken, challenge);
  const { promise, resolve } = Promise.withResolvers<EngineProofAnswer>();
  let answered = false;
  const outgoing = request({
    ...route,
    method: "POST",
    path: ENGINE_PROOF_PATH,
    headers: { [ENGINE_CHALLENGE_HEADER]: challenge, "content-length": 0, Host: "localhost" },
  }, (response) => {
    answered = true;
    const chunks: Buffer[] = [];
    let size = 0;
    response.on("data", (chunk: Buffer) => {
      size += chunk.length;
      if (size > MAX_ANSWER_BYTES) outgoing.destroy(new Error("Engine proof too long"));
      else chunks.push(chunk);
    });
    response.on("end", () => resolve(proofAnswer(response.statusCode, Buffer.concat(chunks), expected)));
    // The first verdict wins: a response cut short ends here without `end`.
    response.on("close", () => resolve("refused"));
  });
  // The silence is the verdict, before the destroy below reports it as an error.
  const limit = setTimeout(() => {
    resolve("silent");
    outgoing.destroy(new Error("Engine proof timed out"));
  }, Math.max(0, answerBy - Date.now()));
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
    outgoing.destroy();
  }
}

/** What a caller may give {@link engineCapabilityProof} in place of its own. */
export interface EngineProofOptions {
  /** The challenge sent: a fresh one of 32 random bytes by default. */
  challenge?: string;
  /** Waits between the tries of a Windows pipe whose instances are all taken; a test counts the tries through it. */
  sleep?: (ms: number) => Promise<void>;
  /**
   * Whether a Windows pipe whose instances are all taken is waited for until one is free, up to {@link INSTANCE_WAIT_MS},
   * with `timeoutMs` counted from the connect, as main's proof waited. Windows fails a connect to a pipe that is gone,
   * so a pipe with every instance taken has a listener that lives and is busy: a hook waits for it before its event.
   * Without it the answer comes within `timeoutMs` of the ask, as a command needs.
   */
  waitForInstance?: boolean;
}

/**
 * Asks the process listening on `socketPath` (a Unix socket or a Windows named pipe) to prove it holds `accessToken`: it
 * answers a fresh challenge with {@link engineProof}. Only the challenge is sent; the capability never leaves this
 * process. The answer comes within `timeoutMs` of the ask, or of the connect with `waitForInstance`, whatever the
 * listener does, and nothing of the ask outlives it: the connect is bounded ({@link proofRoute}), and so is the
 * exchange ({@link askForProof}).
 */
export async function engineCapabilityProof(
  socketPath: string,
  accessToken: string,
  timeoutMs = 2_000,
  { challenge = randomBytes(32).toString("hex"), sleep: wait = sleep, waitForInstance = false }: EngineProofOptions = {},
): Promise<EngineProofAnswer> {
  const asked = Date.now();
  const route = await proofRoute(socketPath, asked + (waitForInstance ? INSTANCE_WAIT_MS : timeoutMs), wait);
  if (typeof route === "string") return route;
  return await askForProof(route, accessToken, challenge, (waitForInstance ? Date.now() : asked) + timeoutMs);
}

/**
 * Whether the process listening on `socketPath` holds `accessToken`, as the engine start asks of a pipe another process
 * holds ({@link engineCapabilityProof}). A Windows pipe whose instances are all taken is waited for up to
 * {@link INSTANCE_WAIT_MS}, and the listener then has `timeoutMs` to answer. A listener that refuses, answers anything
 * else, hangs up, stays silent or keeps every instance taken does not hold it.
 */
export async function engineHoldsCapability(
  socketPath: string,
  accessToken: string,
  timeoutMs = 2_000,
  challenge: string = randomBytes(32).toString("hex"),
): Promise<boolean> {
  return await engineCapabilityProof(socketPath, accessToken, timeoutMs, { challenge, waitForInstance: true }) === "proved";
}
