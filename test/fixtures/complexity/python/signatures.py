def wrapped_signature(
    items,
    limit,
    strict=False,
) -> dict[
    str, int
]:
    total = 0
    for item in items:
        if item is None or not item:
            continue
        if strict and item > limit:
            raise ValueError(item)
        elif item > limit:
            total += limit
        else:
            total += item
    return total


def one_line_signature(items, limit, strict=False):
    total = 0
    for item in items:
        if item is None or not item:
            continue
        if strict and item > limit:
            raise ValueError(item)
        elif item > limit:
            total += limit
        else:
            total += item
    return total


def bracket_default(name, *, bases=(),
                    slots=False):
    if slots and bases:
        return name
    elif slots or not bases:
        return None
    for base in bases:
        if base is None:
            continue
        while base:
            base = base.parent
    return name
