import os.path


def helper(x):
    return leaf(x) + 1


def leaf(x):
    return os.path.join("a", str(x)) and x


def later_defined_caller():
    return defined_later()


def defined_later():
    return 1


def recursive(n):
    if n > 0:
        return recursive(n - 1)
    return 0


def mutual_a(n):
    return mutual_b(n - 1) if n > 0 else 0


def mutual_b(n):
    return mutual_a(n)


def shadowed():
    return 1


def shadowed():  # noqa: F811 - the later definition is the one a call runs
    return 2


def with_default(x=leaf(0)):
    return x


def conditional(flag):
    if flag:
        def pick():
            return "a"
    else:
        def pick():
            return "b"
    return pick()


def run_all():
    helper(1)
    later_defined_caller()
    recursive(2)
    mutual_a(3)
    shadowed()
    with_default()
    conditional(True)
    conditional(False)
    print(len([1]), end="")
    sorted([3, 1], key=leaf)
