from typing import TypedDict

def deco(*a): ...

Pub = TypedDict("Pub", {"x": int for _a in ()})
@deco([0 for _b in ()])
class Pub2(TypedDict):
    x = [0 for _c in ()]
class Pub3(TypedDict, **{k: 1 for _d in ()}):
    y: int
Pub4: int = TypedDict("Pub4", {"x": int for _e in ()})
Pub5 = Pub6 = TypedDict("Pub5", {"x": int for _f in ()})
class Pub7(dict, x=[0 for _g in ()]): ...
class Pub8[T: [0 for _h in ()]](TypedDict): ...
