import os


def f(a: "os.path.samefile('a', 'b')") -> "os.path.samefile(a, a) or os.path.samefile(a, a)":
    return os.path.samefile()


x = "os.path.samefile('a', 'b')"
