import contextlib


def plain():
    a = 1
    b = 2
    print(b)
    c = 3
    c = 4
    d = 5
    print(d)
    d = 6
    e: int = 7
    f: int
    (g) = 8
    _ = 9
    _dummy = 10
    __tracebackhide__ = True


def augmented_and_walrus():
    h = 0
    h += 1
    if (i := len([])) > 0:
        pass
    print([j := 1 for _ in range(3)])


def unpacking(pair):
    k, m = pair
    (n, o) = 1, 2
    [p, q] = [3, 4]
    r = s = 5
    t, u = v = pair
    w1 = x1, y1 = 6, 7
    return s


def contexts():
    with contextlib.suppress(Exception) as w:
        pass
    with open("a") as x, open("b") as y:
        print(y)
    with open("c") as (z1, z2):
        pass
    try:
        pass
    except ValueError as err:
        pass
    try:
        pass
    except ValueError as used_err:
        print(used_err)
    for loop_var in range(3):
        pass


def scopes():
    outer_value = 1
    assigned_later = 1

    def inner():
        return outer_value

    def rebinds():
        nonlocal assigned_later
        assigned_later = 2

    global module_value
    module_value = 3
    return inner, rebinds


def deletion():
    removed = 1
    del removed
    kept = 1
    if kept:
        del kept


def dynamic():
    hidden = 1
    return locals()


def dynamic_eval():
    seen = 1
    return eval("seen")


def matched(value):
    match value:
        case [first, *rest]:
            pass
        case {"key": captured}:
            pass
        case str() as whole:
            pass


lambda_value = lambda: (lam := 1)


class Klass:
    attribute = 1

    def method(self):
        local = self
        return self


def locals_hides_only_its_own_assignments(parameter):
    hidden_by_locals = 1
    try:
        pass
    except ValueError as caught_despite_locals:
        pass
    return locals()


def locals_in_a_nested_scope():
    reported_outer = 1
    kept = [locals() for _ in range(1)]

    def inner():
        return locals()

    return inner, kept


def other_dynamic_builtins():
    by_vars = 1
    by_exec = 1
    by_globals = 1
    vars()
    exec("by_exec")
    return globals()


lambda_locals = lambda: (in_lambda := 1, locals())


try:
    pass
except ValueError as module_caught:
    pass


class HasACell:
    def rebinds_the_cell(self):
        nonlocal __class__
        __class__ = 1

    def binds_its_own(self):
        __class__ = 2


def later_clause_reads_the_enclosing_binding():
    shadowed_by_a_clause = 1
    return [y for y in range(3) for shadowed_by_a_clause in shadowed_by_a_clause]


def an_except_name_shadows_a_local():
    shadowed_by_the_handler = 1
    print(shadowed_by_the_handler)
    try:
        pass
    except ValueError as shadowed_by_the_handler:
        pass
    return shadowed_by_the_handler


def read_only_after_the_handler():
    read_late = 1
    try:
        pass
    except ValueError as read_late:
        pass
    return read_late
