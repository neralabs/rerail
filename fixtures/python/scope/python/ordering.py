# Evaluation order, not text order: a comprehension's element runs after its clauses, a
# conditional expression's test before its body, an assignment's value before its targets.
chunks = [text for block in blocks if (text := block.text)]
label = named if (named := compute()) else fallback_name
first = [inner for inner in range(3) if inner][0] if (limit := 3) else limit
value = value_before + 1
value_before = 2
row, column = column_later, row
column_later = 0


class Later:
    pass


def uses_later(x: Later, y: NeverDefined) -> Later:
    return x, y


def refers_to_class_later() -> list[AlsoLater]:
    return []


class AlsoLater:
    pass


def clause_order():
    own = 1
    return [x for x in own for own in own if own], [z for z in own for w in w]
