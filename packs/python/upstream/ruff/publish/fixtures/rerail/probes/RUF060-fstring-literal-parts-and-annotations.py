from typing import cast
a = "a"
r1 = a in f"{'x' f''}"
r2 = a in f"{f'' 'x'}"
r3 = a in f"{'x' 'y'}"
r4 = a in f"\
{''}"
r5 = a in rf"\
"
x: "1 in []" = 1
y = cast("1 not in list()", 1)
z: "list['1 in ()']" = []
w = "1 in []"
