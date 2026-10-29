import sys


class C:
    sys = None
    a = [sys.version_info.minor > 5 for _ in range(3)]
    b = [x for x in [sys.version_info.minor > 5]]
