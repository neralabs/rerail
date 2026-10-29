from typing import Dict
xs = [(1, 2)]
a: "dict((k, v) for k, v in xs)"
b: "Dict['dict((k,) for k in xs)', int]"
c = "dict((k, v) for k, v in xs)"


class C:
    dict = list
    e: "dict((k, v) for k, v in xs)"
