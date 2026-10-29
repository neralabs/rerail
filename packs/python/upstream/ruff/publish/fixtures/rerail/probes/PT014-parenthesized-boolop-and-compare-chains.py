import pytest
a = b = c = 1

@pytest.mark.parametrize("x", [(a and b) and c, a and b and c, a and (b and c), (a or b) or c, a or b or c, a or (b or c)])
def t1(x): ...

@pytest.mark.parametrize("x", [(a < b) < c, a < b < c, a < (b < c), (a < b) < c])
def t2(x): ...

@pytest.mark.parametrize("x", [not not a, not (not a), - - a, -(-a), a + b + c, (a + b) + c, a + (b + c)])
def t3(x): ...
