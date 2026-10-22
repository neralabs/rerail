from typing import TypeAlias
import typing

Pub: TypeAlias = list[[0 for _Q in ()]]
type T1 = [0 for _R in ()]
x.y: TypeAlias = [0 for _S in ()]
Pub2: TypeAlias = [_U for _U in ()]
Pub3: TypeAlias = [0 for _V in () for _V in ()]
Pub4: TypeAlias = (lambda: [0 for _W in ()])
Pub5: int = [0 for _X in ()]
Pub6: TypeAlias = [0 for _Y, _Z in ()]
class K:
    Pub7: TypeAlias = {0: 1 for _A in ()}
    x = [0 for _B in ()]
Pub8: TypeAlias = [0 for q in () if (_C := 1)]
Pub9: TypeAlias = [[0 for _D in ()] for e in ()]
(Pub10): TypeAlias = [0 for _E in ()]
