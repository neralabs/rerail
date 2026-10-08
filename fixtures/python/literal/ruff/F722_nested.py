a: "(yield z)"
b: "list['(yield z)']"
c: "list[list['(yield z)']]"
d: "tuple['(yield y)', '(await w)']"
e: "list['int', '(yield v)']"
f: "list['int']"
g: "dict[str, 'list[\"(yield t)\"]']"


def h(x: "list['(yield s)']") -> "list['int']":
    return x
