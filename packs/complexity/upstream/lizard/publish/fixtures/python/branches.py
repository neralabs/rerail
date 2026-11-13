"""Statement decisions: if/elif, loops, except/finally and boolean operators, at and over the limit."""


def if_elif_at_limit(x):
    if x == 1:
        return "one"
    elif x == 2:
        return "two"
    elif x == 3:
        return "three"
    elif x == 4:
        return "four"
    elif x == 5:
        return "five"
    return "many"


def if_elif_over_limit(x):
    if x == 1:
        return "one"
    elif x == 2:
        return "two"
    elif x == 3:
        return "three"
    elif x == 4:
        return "four"
    elif x == 5:
        return "five"
    elif x == 6:
        return "six"
    return "many"


def loops_at_limit(rows):
    total = 0
    for row in rows:
        total += row
    while total > 100:
        total -= 1
    for row in rows:
        for cell in row:
            total += cell
    while total < 0:
        total += 1
    return total


def loops_over_limit(rows):
    total = 0
    for row in rows:
        total += row
    while total > 100:
        total -= 1
    for row in rows:
        for cell in row:
            for part in cell:
                total += part
    while total < 0:
        total += 1
    return total


async def async_loops_over_limit(stream, rows):
    total = 0
    async for item in stream:
        total += item
    async for item in stream:
        total -= item
    for row in rows:
        total += row
    while total > 10:
        total -= 1
    for row in rows:
        total += row
    while total < 0:
        total += 1
    return total


def exceptions_at_limit(action):
    try:
        return action()
    except KeyError:
        return "key"
    except ValueError:
        return "value"
    except TypeError:
        return "type"
    except OSError:
        return "os"
    finally:
        print("done")


def exceptions_over_limit(action):
    try:
        return action()
    except KeyError:
        return "key"
    except ValueError:
        return "value"
    except TypeError:
        return "type"
    except OSError:
        return "os"
    except RuntimeError:
        return "runtime"
    finally:
        print("done")


def boolean_at_limit(a, b, c, d, e, f):
    return a and b or c and d or e and f


def boolean_over_limit(a, b, c, d, e, f, g):
    return a and b or c and d or e and f and g


def uncounted_constructs_at_limit(path, items):
    with open(path) as handle:
        assert handle
        try:
            handle.read()
        except OSError:
            return None
        else:
            pass
    for item in items:
        if item:
            continue
    else:
        pass
    while items:
        items.pop()
    else:
        pass
    if path:
        return path
    else:
        return None
