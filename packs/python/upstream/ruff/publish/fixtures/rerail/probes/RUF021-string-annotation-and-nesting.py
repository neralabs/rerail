from typing import cast

a, b, c, d = 1, 0, 2, 3
x: "a or b and c" = 1
y = cast("a and b or c", 1)
z = a or (b and c) or d and a
w = f"{a or b and c}"
v = a and (b or c and a)
u = (a or b) and c
t = a or not b and c
