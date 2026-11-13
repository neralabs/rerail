// Encoding checks for the transducer language in docs/transducer-design.md, not an execution backend. Runs under
// `node --test` (`npm run test:scripts`). A word is opcode | mode << 8 | operand << 16; it is built and taken apart
// with arithmetic, since JavaScript's bitwise operators work on signed 32-bit integers and an operand of 0x8000 or
// more would come out negative.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const DESIGN = new URL("../../docs/transducer-design.md", import.meta.url);
const OPCODES = new Set([0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x10, 0x11, 0x12, 0x13, 0x20, 0x30, 0x31, 0x40]);
const WORD_MAX = 0xffffffff;

const inRange = (value, max) => Number.isInteger(value) && value >= 0 && value <= max;

function decode(word) {
  if (!inRange(word, WORD_MAX)) throw new RangeError("instruction must fit one word");
  const [opcode, mode, operand] = [word % 0x100, Math.floor(word / 0x100) % 0x100, Math.floor(word / 0x10000)];
  if (!OPCODES.has(opcode)) throw new RangeError("invalid opcode");
  if (opcode === 0x03) {
    if (mode & ~15 || operand & ~15 || mode & operand) throw new RangeError("invalid flags");
  } else if (opcode === 0x05) {
    if (![0, 1, 2, 0x80, 0x81, 0x82].includes(mode)) throw new RangeError("invalid size mode");
  } else if ([0x10, 0x11, 0x13].includes(opcode)) {
    if (![0, 1].includes(mode) || operand) throw new RangeError("invalid navigation mode");
  } else if (mode || ([0x00, 0x12].includes(opcode) && operand)) {
    throw new RangeError("nonzero unused field");
  }
  return [opcode, mode, operand];
}

function encode(opcode, mode = 0, operand = 0) {
  if (!(inRange(opcode, 0xff) && inRange(mode, 0xff) && inRange(operand, 0xffff))) throw new RangeError("instruction field overflow");
  const word = opcode + mode * 0x100 + operand * 0x10000;
  decode(word);
  return word;
}

/** `word` as the four little-endian bytes it occupies; throws when it does not fit them. */
function packed(word) {
  const bytes = Buffer.alloc(4);
  bytes.writeUInt32LE(word);
  return bytes;
}

/** The design's text with `\n` line ends, whatever line ends the checkout wrote. */
const designText = () => readFileSync(DESIGN, "utf8").replace(/\r\n?/gu, "\n");

/** The design's example program: each row's position and word. */
function publishedExample() {
  const block = designText().split("position  instruction          word")[1].split("```")[0];
  return [...block.matchAll(/^(\d+)\s+.*?\s+([0-9A-Fa-f]{8})$/gmu)].map(([, position, word]) => [Number(position), Number.parseInt(word, 16)]);
}

test("the example is eight single-word instructions", () => {
  const rows = publishedExample();
  assert.deepEqual(rows.map(([position]) => position), [0, 1, 2, 3, 4, 5, 6, 7]);
  assert.deepEqual(rows.map(([, word]) => decode(word)), [
    [0x01, 0, 0],
    [0x20, 0, 1],
    [0x10, 0, 0],
    [0x02, 0, 1],
    [0x01, 0, 1],
    [0x04, 0, 0],
    [0x20, 0, 0],
    [0x40, 0, 2],
  ]);
  assert.equal(Buffer.concat(rows.map(([, word]) => packed(word))).length, 32);
});

test("the operand boundary has no escape", () => {
  for (const operand of [0, 32767, 32768, 65534, 65535]) {
    const word = encode(0x31, 0, operand);
    assert.deepEqual(decode(word), [0x31, 0, operand]);
    assert.equal(packed(word).length, 4);
  }
});

test("a long constant is one word", () => {
  const text = designText();
  const word = Number.parseInt(/SIZE at-least,long 3 = ([0-9A-Fa-f]{8})/u.exec(text)[1], 16);
  assert.equal(word, encode(0x05, 0x81, 3));
  assert.deepEqual(decode(word), [0x05, 0x81, 3]);
  const longs = [0, 0, 0, 100000];
  assert.equal(longs[decode(word)[2]], 100000);
  assert.equal(packed(word).length, 4);
});

test("a large inline value is rejected", () => {
  assert.throws(() => encode(0x05, 1, 100000), RangeError);
});

test("a field or word overflow is rejected", () => {
  for (const args of [[256, 0, 0], [1, 256, 0], [1, 0, 65536], [1, 0, -1]]) {
    assert.throws(() => encode(...args), RangeError, `encode(${args.join(", ")})`);
  }
  for (const word of [-1, 2 ** 32]) assert.throws(() => decode(word), RangeError, `decode(${word})`);
});

test("invalid modes and reserved fields are rejected", () => {
  for (const [opcode, mode, operand] of [
    [0xff, 0, 0],
    [0x01, 0x80, 0],
    [0x05, 3, 0],
    [0x10, 0, 1],
    [0x00, 0, 1],
    [0x03, 1, 1],
    [0x03, 16, 0],
  ]) {
    assert.throws(() => decode(opcode + mode * 0x100 + operand * 0x10000), RangeError, `decode(${opcode}, ${mode}, ${operand})`);
  }
});
