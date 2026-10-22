r = [2, 1]
list: type
a = list([i for i in r])


def g():
    global list
    list = print


def h():
    return list([i for i in r])


def k():
    with open(__file__):
        del list
    return list([i for i in r])
