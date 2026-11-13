import { createHmac, randomBytes, timingSafeEqual } from "node:crypto";
import { request } from "node:http";

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
 * Asks the process listening on `socketPath` (a Unix socket or a Windows named pipe) to prove it holds `accessToken`: it
 * answers a fresh challenge with {@link engineProof}. Only the challenge is sent; the capability never leaves this
 * process. The answer comes within `timeoutMs` of the ask, whatever the listener does: the limit is a timer of its own,
 * since a request's timeout starts only once its socket has connected and restarts at each byte read. On Windows a
 * connect to a pipe whose instances are all taken waits up to 30 s for a free one, and an engine whose event loop is
 * stuck frees none.
 */
export async function engineCapabilityProof(
  socketPath: string,
  accessToken: string,
  timeoutMs = 2_000,
  challenge: string = randomBytes(32).toString("hex"),
): Promise<EngineProofAnswer> {
  const expected = engineProof(accessToken, challenge);
  const { promise, resolve } = Promise.withResolvers<EngineProofAnswer>();
  let answered = false;
  const outgoing = request({
    socketPath,
    method: "POST",
    path: ENGINE_PROOF_PATH,
    agent: false,
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
  }, timeoutMs);
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
