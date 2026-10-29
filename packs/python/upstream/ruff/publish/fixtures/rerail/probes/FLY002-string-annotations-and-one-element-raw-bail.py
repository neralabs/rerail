from typing import List, Literal, cast
a = b = None
x1: "''.join([a, b]) | '-'.join((a, 'x'))" = None
x2: List["''.join([a, b])"] = []
x3 = cast("''.join(['p', 'q'])", a)
x4: Literal["''.join([a, b])"] = None
x5 = "''.join([a, b])"
x6: "''.join(" "[a, b])" = None
x7: "''.join([a, \"'\", '\"'])" = None
y1 = '\n'.join([r'a"""' "'''"])
y2 = "'''\n".join([r'a"""'])
y3 = "'''\n".join([r'a"""', 'b'])
y4 = '"""'.join([r'''a
''' "'''"])
y5 = '"""'.join([r'''a
''', "'''"])
