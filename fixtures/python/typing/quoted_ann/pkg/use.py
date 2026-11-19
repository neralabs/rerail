from pkg.model import Node, make


def build(n: "Node") -> str:
    return n.label()


def walk() -> str:
    m = make()
    return m.child().label()
