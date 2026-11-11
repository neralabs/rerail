def missing(values):
    return [v for v in values if v == None]


def present(value):
    if value != None:
        return value
    return None


def missing_right(values):
    return [v for v in values if v is None]
