from foo import log1, log3

x = 1


def f():
    log1.info("a" + str(x))


log1 = None
log2 = None


def g():
    log2.info("a" + str(x))


from foo import log2


class C:
    log3 = None
    log3.info("a" + str(x))

    def m(self):
        log3.info("a" + str(x))

    inner = [log3.info("a" + str(x)) for _ in range(1)]
    first = [y for y in log3.info("a" + str(x))]
