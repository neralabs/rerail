from typing import TypedDict


class Outer:
    if True:
        class CondTD(TypedDict):
            x: int


class FromNested(Outer.CondTD):
    nestedCase: int


class DoubleSub(TypedDict[int][str]):
    doubleCase: int


class A:
    def f(self, x: (aB := int) = (cD := 1)) -> (eF := int):
        pass
    xs = [x for x in (gH := range(3)) if (iJ := x) for y in (kL := xs)]
    type mN = (oP := int)
    with ctx() as (qR): pass
    with ctx() as (sT, *uV): pass
    match xs:
        case [wX, (yZ | yZ)]: pass
