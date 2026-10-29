from typing import Literal
a: "Literal[1]" ""
b: r"Literal[\x31]"
c: "Literal[\q1]"
d: u"Literal[\x31]"
e: "1" "2"
f: "list['Literal[1]' '']"
g: ("Literal[1]"
    "")
h: "Literal[12345678901]" ""
