def plain(y):
    i = 0
    for x in y:
        i += 1


def unpacked(y):
    i, j = 0, 1
    for x in y:
        i += 1
    k, m = 1, 0
    for x in y:
        m += 1


def not_zero(y):
    i = 1
    for x in y:
        i += 1


def two_identical_loops(y):
    i = 0
    for x in y:
        i += 1
    for x in y:
        i += 1


def two_identical_loops_then_read(y):
    i = 0
    for x in y:
        i += 1
    print(i)
    j = 0
    for x in y:
        j += 1


def read_after(y):
    i = 0
    for x in y:
        i += 1
    return i


def read_before(y):
    i = 0
    print(i)
    for x in y:
        i += 1


def nested_level(y, c):
    i = 0
    if c:
        for x in y:
            i += 1


def with_continue(y):
    i = 0
    for x in y:
        if x:
            continue
        i += 1


def twice(y):
    i = 0
    for x in y:
        i += 1
        i += 1


def by_two(y):
    i = 0
    for x in y:
        i += 2


def walrus_in_comprehension(y):
    i = 0
    for x in y:
        i += 1
        print([(i := z) for z in x])


def match_capture(y):
    i = 0
    for x in y:
        i += 1
        match x:
            case [i]:
                pass


def deleted(y):
    i = 0
    for x in y:
        i += 1
        del i


def comprehension_target(y):
    i = 0
    for x in y:
        i += 1
        print([i for i in x])


def keyword_argument(y):
    i = 0
    for x in y:
        i += 1
        print(x, i=2)


def nested_function(y):
    i = 0
    for x in y:
        i += 1

        def g():
            i = 5
            return i
