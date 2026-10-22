import enum
import sys


class Outer:
    if sys.version_info >= (3, 8):
        class Inner(enum.Enum):
            pass

    class Gone(enum.Enum):
        pass

    Gone = 1

    class Deep:
        class Deeper(enum.Flag):
            pass

    def method(self):
        class Hidden(enum.Enum):
            pass


class InsideIf(str, Outer.Inner):  # a member bound under `if` still counts
    pass


class Rebound(str, Outer.Gone):  # the live binding is no longer the enum
    pass


class TwoLevels(str, Outer.Deep.Deeper):
    pass


class FunctionLocal(str, Outer.Hidden):  # method locals are not class members
    pass
