# Nine counted cases, then a last case without a guard: an irrefutable one costs nothing (complexity 10, silent),
# a refutable one costs 1 (complexity 11, reported at the default limit of 10).


def wild_as(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case _ as z: pass


def capture_as(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case y as z: pass


def group_as(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case (y as z): pass


def or_wild_as(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case (0 | (_)) as z: pass


def value_as(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case 1 as z: pass


def none(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case None: pass


def dotted(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case a.b: pass


def one_tuple(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case y,: pass


def star_list(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case [*_]: pass


def values_as(x):
    match x:
        case 0: pass
        case 1: pass
        case 2: pass
        case 3: pass
        case 4: pass
        case 5: pass
        case 6: pass
        case 7: pass
        case 8: pass
        case (1 | 2) as z: pass
