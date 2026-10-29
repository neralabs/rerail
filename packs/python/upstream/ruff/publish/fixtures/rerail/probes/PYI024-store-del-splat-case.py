import collections
from collections import namedtuple
del collections.namedtuple
collections.namedtuple += 1
namedtuple += 1
(namedtuple) -= 1
x = 1
x += namedtuple
a = [*collections.namedtuple("a", "b")._fields]
b = *collections.namedtuple, 1
match v:
    case collections.namedtuple.attr.deep:
        pass
    case collections.namedtuple2:
        pass
