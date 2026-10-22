from foo import log1, log2, log3, log4, log5, log6, log7

x = 1
if x:
    del log1
log1.info("%s" % x)
for _ in []:
    del log2
log2.info("%s" % x)
try:
    pass
except Exception:
    del log3
log3.info("%s" % x)
try:
    pass
except Exception as log4:
    pass
log4.info("%s" % x)
try:
    pass
except Exception as log_err:
    pass
log_err.info("%s" % x)


def f():
    log5: int
    log5.info("%s" % x)


def g():
    global log6
    log6.info("%s" % x)


def h():
    log7 = None

    def inner():
        nonlocal log7
        log7.info("%s" % x)
