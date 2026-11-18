import { describe, expect, it } from "vitest";
import { EvaluationSlot } from "../../../src/engine/native/evaluation-slot.js";

/** The engine's refusal of a call that finds its slot held, as these tests raise it. */
class Refused extends Error {}
const refused = (error: unknown) => error instanceof Refused;

/** Lets every promise reaction already queued run, through one turn of the host's loop (no clock). */
const settle = () => new Promise<void>(resolve => setImmediate(resolve));

/** A call that holds the slot until `finish` is called, recording when it starts. */
function heldCall(name: string, started: string[]) {
  const done = Promise.withResolvers<string>();
  return {
    task: () => {
      started.push(name);
      return done.promise;
    },
    finish: () => done.resolve(name),
    fail: () => done.reject(new Error(`${name} failed`)),
  };
}

describe("the evaluation slot", () => {
  it("runs calls in the order they ask, each once the one before it has settled", async () => {
    const slot = new EvaluationSlot(refused);
    const signal = new AbortController().signal;
    const started: string[] = [];
    const first = heldCall("first", started);
    const second = heldCall("second", started);
    const third = heldCall("third", started);
    const results = [first, second, third].map(call => slot.run(call.task, signal));
    await settle();
    expect(started).toEqual(["first"]);
    expect(slot.idle).toBe(false);
    first.finish();
    await settle();
    expect(started).toEqual(["first", "second"]);
    second.finish();
    third.finish();
    expect(await Promise.all(results)).toEqual(["first", "second", "third"]);
    expect(slot.idle).toBe(true);
  });

  it("takes a waiting call out of the line when its signal aborts, and never runs it", async () => {
    const slot = new EvaluationSlot(refused);
    const started: string[] = [];
    const holder = heldCall("holder", started);
    const leaving = heldCall("leaving", started);
    const next = heldCall("next", started);
    const closing = new AbortController();
    const held = slot.run(holder.task, new AbortController().signal);
    const left = slot.run(leaving.task, closing.signal);
    const after = slot.run(next.task, new AbortController().signal);
    closing.abort(new Error("session closed"));
    await expect(left).rejects.toThrow("session closed");
    holder.finish();
    await settle();
    next.finish();
    expect(await Promise.all([held, after])).toEqual(["holder", "next"]);
    expect(started).toEqual(["holder", "next"]);
    expect(slot.idle).toBe(true);
  });

  it("never runs a call whose signal aborted before it asked, even with the slot free", async () => {
    const slot = new EvaluationSlot(refused);
    const started: string[] = [];
    const closed = new AbortController();
    closed.abort(new Error("session closed"));
    await expect(slot.run(heldCall("late", started).task, closed.signal)).rejects.toThrow("session closed");
    expect(started).toEqual([]);
    expect(slot.idle).toBe(true);
  });

  it("hands the slot on when a call fails", async () => {
    const slot = new EvaluationSlot(refused);
    const signal = new AbortController().signal;
    const started: string[] = [];
    const failing = heldCall("failing", started);
    const next = heldCall("next", started);
    const failed = slot.run(failing.task, signal);
    const after = slot.run(next.task, signal);
    failing.fail();
    await expect(failed).rejects.toThrow("failing failed");
    await settle();
    next.finish();
    expect(await after).toBe("next");
    expect(slot.idle).toBe(true);
  });

  it("runs a call beside the line at once, and a call that asks meanwhile does not wait for it", async () => {
    const slot = new EvaluationSlot(refused);
    const signal = new AbortController().signal;
    const started: string[] = [];
    const planning = heldCall("planning", started);
    const check = heldCall("check", started);
    const beside = slot.runBeside(planning.task, signal);
    const line = slot.run(check.task, signal);
    await settle();
    expect(started).toEqual(["planning", "check"]);
    check.finish();
    expect(await line).toBe("check");
    planning.finish();
    expect(await beside).toBe("planning");
    expect(slot.idle).toBe(true);
  });

  it("asks again for a call the engine refused while a call beside the line held its slot, once that call settles", async () => {
    const slot = new EvaluationSlot(refused);
    const signal = new AbortController().signal;
    const started: string[] = [];
    const batch = heldCall("batch", started);
    const beside = slot.runBeside(batch.task, signal);
    let asked = 0;
    const line = slot.run(() => {
      asked += 1;
      return asked === 1 ? Promise.reject(new Refused("slot held")) : Promise.resolve("check");
    }, signal);
    await settle();
    expect(asked).toBe(1);
    batch.finish();
    expect(await line).toBe("check");
    expect(asked).toBe(2);
    expect(await beside).toBe("batch");
  });

  it("gives a refusal no call beside the line explains to its caller, and asks no more", async () => {
    const slot = new EvaluationSlot(refused);
    let asked = 0;
    const line = slot.run(() => {
      asked += 1;
      return Promise.reject(new Refused("held outside the line"));
    }, new AbortController().signal);
    await expect(line).rejects.toThrow("held outside the line");
    expect(asked).toBe(1);
    expect(slot.idle).toBe(true);
  });

  it("puts a call beside the line that the engine refused into the line, behind the calls already there", async () => {
    const slot = new EvaluationSlot(refused);
    const signal = new AbortController().signal;
    const started: string[] = [];
    const holder = heldCall("holder", started);
    const held = slot.run(holder.task, signal);
    let asked = 0;
    const beside = slot.runBeside(() => {
      asked += 1;
      return asked === 1 ? Promise.reject(new Refused("slot held")) : Promise.resolve("planned");
    }, signal);
    await settle();
    expect(asked).toBe(1);
    holder.finish();
    expect(await held).toBe("holder");
    expect(await beside).toBe("planned");
    expect(asked).toBe(2);
    expect(slot.idle).toBe(true);
  });
});
