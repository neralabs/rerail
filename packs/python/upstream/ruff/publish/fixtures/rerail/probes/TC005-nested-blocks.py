from typing import TYPE_CHECKING


def f():
    if TYPE_CHECKING:
        pass


class C:
    if TYPE_CHECKING:
        pass

    def m(self):
        try:
            if TYPE_CHECKING:
                pass
        finally:
            pass
        with open("x") as fh:
            if TYPE_CHECKING:
                pass
        for i in range(3):
            if TYPE_CHECKING:
                pass
        while True:
            if TYPE_CHECKING:
                pass
            break


async def g():
    if TYPE_CHECKING:
        pass


@decorator
def h():
    if TYPE_CHECKING:
        pass
    elif x:
        pass
    else:
        pass


match x:
    case 1:
        if TYPE_CHECKING:
            pass
lambda: (yield)
