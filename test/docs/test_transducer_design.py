#!/usr/bin/env python3
"""Encoding checks for the transducer language, not an execution backend."""

from pathlib import Path
import re
import struct
import unittest

DESIGN = Path(__file__).resolve().parents[2] / "docs/transducer-design.md"
OPCODES = {
    0x00,
    0x01,
    0x02,
    0x03,
    0x04,
    0x05,
    0x10,
    0x11,
    0x12,
    0x13,
    0x14,
    0x20,
    0x30,
    0x31,
    0x32,
    0x33,
    0x40,
}


def encode(opcode, mode=0, operand=0):
    if not (0 <= opcode <= 0xFF and 0 <= mode <= 0xFF and 0 <= operand <= 0xFFFF):
        raise ValueError("instruction field overflow")
    word = opcode | (mode << 8) | (operand << 16)
    decode(word)
    return word


def decode(word):
    if not 0 <= word <= 0xFFFFFFFF:
        raise ValueError("instruction must fit one word")
    opcode, mode, operand = word & 0xFF, (word >> 8) & 0xFF, word >> 16
    if opcode not in OPCODES:
        raise ValueError("invalid opcode")
    if opcode == 0x03:
        if mode & ~15 or operand & ~15 or mode & operand:
            raise ValueError("invalid flags")
    elif opcode == 0x05:
        if mode not in (0, 1, 2, 0x80, 0x81, 0x82):
            raise ValueError("invalid size mode")
    elif opcode in (0x10, 0x11, 0x13, 0x14):
        if mode not in (0, 1) or operand:
            raise ValueError("invalid navigation mode")
    elif opcode in (0x02, 0x32):
        if mode not in (0, 1):
            raise ValueError("invalid mode")
    elif mode or (opcode in (0x00, 0x12) and operand):
        raise ValueError("nonzero unused field")
    return opcode, mode, operand


def published_example():
    block = (
        DESIGN.read_text()
        .split("position  instruction          word", 1)[1]
        .split("```", 1)[0]
    )
    rows = re.findall(r"^(\d+)\s+.*?\s+([0-9A-Fa-f]{8})$", block, re.MULTILINE)
    return [(int(position), int(word, 16)) for position, word in rows]


class EncodingTests(unittest.TestCase):
    def test_example_is_eight_single_word_instructions(self):
        rows = published_example()
        self.assertEqual([position for position, _ in rows], list(range(8)))
        self.assertEqual(
            [decode(word) for _, word in rows],
            [
                (0x01, 0, 0),
                (0x20, 0, 1),
                (0x10, 0, 0),
                (0x02, 0, 1),
                (0x01, 0, 1),
                (0x04, 0, 0),
                (0x20, 0, 0),
                (0x40, 0, 2),
            ],
        )
        self.assertEqual(len(b"".join(struct.pack("<I", word) for _, word in rows)), 32)

    def test_operand_boundary_has_no_escape(self):
        for operand in (0, 32767, 32768, 65534, 65535):
            word = encode(0x31, operand=operand)
            self.assertEqual(decode(word), (0x31, 0, operand))
            self.assertEqual(len(struct.pack("<I", word)), 4)

    def test_long_constant_is_one_word(self):
        text = DESIGN.read_text()
        word = int(re.search(r"SIZE at-least,long 3 = ([0-9A-Fa-f]{8})", text)[1], 16)
        self.assertEqual(word, encode(0x05, 0x81, 3))
        self.assertEqual(decode(word), (0x05, 0x81, 3))
        longs = [0, 0, 0, 100000]
        self.assertEqual(longs[decode(word)[2]], 100000)
        self.assertEqual(len(struct.pack("<I", word)), 4)

    def test_large_inline_value_is_rejected(self):
        with self.assertRaises(ValueError):
            encode(0x05, 1, 100000)

    def test_field_and_word_overflow(self):
        for args in ((256, 0, 0), (1, 256, 0), (1, 0, 65536), (1, 0, -1)):
            with self.subTest(args=args), self.assertRaises(ValueError):
                encode(*args)
        for word in (-1, 1 << 32):
            with self.assertRaises(ValueError):
                decode(word)

    def test_invalid_modes_and_reserved_fields(self):
        for opcode, mode, operand in (
            (0xFF, 0, 0),
            (0x01, 0x80, 0),
            (0x05, 3, 0),
            (0x10, 0, 1),
            (0x00, 0, 1),
            (0x03, 1, 1),
            (0x03, 16, 0),
        ):
            with (
                self.subTest(opcode=opcode, mode=mode, operand=operand),
                self.assertRaises(ValueError),
            ):
                decode(opcode | (mode << 8) | (operand << 16))


if __name__ == "__main__":
    unittest.main()
