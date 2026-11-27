from typing import Never


def bail(msg: str):
    raise ValueError(msg)


# Two levels: the last statement calls a helper that raises.
def reject_twice(msg: str):
    print("rejecting", msg)
    bail(msg)


def never(msg: str) -> Never:
    print(msg)
    raise ValueError(msg)


class Errors:
    @staticmethod
    def invalid(msg: str):
        raise ValueError(msg)


# A look-alike named like a raising helper that only logs.
def fail_soft(msg: str) -> None:
    print("would fail:", msg)
