from os import curdir, path, sep


def reads_after_an_unpacking(source):
    path, size = source.path, source.size()
    [first] = path.split(size)
    return first


def augments_after_an_unpacking(source, totals):
    curdir, size = source.curdir, source.size()
    [totals][0] += curdir
    return size


def reads_before_an_unpacking(source):
    [first] = sep.split(source)
    sep, size = source.sep, source.size()
    return first, size
