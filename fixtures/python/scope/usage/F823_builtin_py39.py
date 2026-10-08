# target-version: py39
def reads_a_later_builtin():
    print(aiter)
    aiter = 1


def reads_an_old_builtin():
    print(len)
    len = 1
