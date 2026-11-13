"""Conditional expressions and comprehension clauses each count once."""


def ternary_at_limit(a, b, c, d, e):
    first = 1 if a else 0
    second = 1 if b else 0
    third = 1 if c else 0
    fourth = 1 if d else 0
    fifth = 1 if e else 0
    return first + second + third + fourth + fifth


def ternary_over_limit(a, b, c, d, e, f):
    first = 1 if a else 0
    second = 1 if b else 0
    third = 1 if c else 0
    fourth = 1 if d else 0
    fifth = 1 if e else 0
    sixth = 1 if f else 0
    return first + second + third + fourth + fifth + sixth


def comprehension_at_limit(rows, columns):
    pairs = [(row, column) for row in rows for column in columns if row if column]
    return {row for row in rows}, pairs


def comprehension_over_limit(rows, columns):
    pairs = [(row, column) for row in rows for column in columns if row if column]
    totals = {row: sum(row) for row in rows if row}
    return list(pair for pair in pairs if pair), totals


async def async_comprehension_over_limit(stream, rows):
    seen = [item async for item in stream if item]
    kept = [row for row in rows if row]
    flags = [1 if row else 0 for row in rows]
    return seen, kept, flags
