def count(values):
    total = 0
    for value in values:
        total += 1
    return total


def pairs(mapping):
    for key, value in mapping.items():
        print(key)


def count_ok(values):
    return sum(1 for _ in values)
