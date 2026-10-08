from os import *

try:
    pass
except ValueError as unbound_after_its_handler:
    pass
print(unbound_after_its_handler)


def reads_it_later():
    return unbound_after_its_handler


rebound = 1
try:
    pass
except ValueError as rebound:
    pass
print(rebound, from_the_star)

try:
    pass
except ValueError as frozenset:
    pass
print(frozenset)

removed = 1
del removed
print(removed)


def annotated_under_a_star():
    from_the_star: int
    return from_the_star
