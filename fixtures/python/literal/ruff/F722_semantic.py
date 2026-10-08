"""String type definitions Ruff's parser accepts and its checker reports semantic syntax errors in, by host scope."""

x: "await y" = 1
x: "(yield y)" = 1
x: "(yield from y)" = 1
x: "f(await y)" = 1
x: "List[await y]" = 1
x: """await y""" = 1
x: "(__debug__ := 1)" = 1
x: "[a async for a in b]" = 1
x: "(a async for a in b)" = 1
x: "[a for a in b if (a := 1)]" = 1
x: "[a for a in (d := e)]" = 1
x: "[await a for a in b]" = 1
x: "(await a for a in b)" = 1
x: "lambda: await y" = 1
x: "lambda: (yield)" = 1
x: "lambda a, a: 1" = 1
x: "f(a=1, a=2)" = 1
x: "(yield *y)" = 1
x: "yield y" = 1
x: "await await y" = 1
x: "f(yield y)" = 1


async def g():
    x: "await y" = 1
    x: "(yield)" = 1
    x: "[a async for a in b]" = 1
    x: "(yield from y)" = 1


def k():
    x: "await y" = 1
    x: "(yield y)" = 1
    x: "a" = 1
    global a


class C:
    x: "await y" = 1
    x: "[(q := 1) for a in b]" = 1


def m(p: "await y" = 1): pass


async def n(p: "await y" = 1) -> "await r": pass


from typing import no_type_check


@no_type_check
def o():
    x: "await y" = 1
