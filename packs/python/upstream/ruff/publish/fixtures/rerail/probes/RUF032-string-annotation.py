from decimal import Decimal
import typing

x: "Decimal((1.5), 2.0)" = 1
y: "list['Decimal(-+2.5)']" = []
z: "Decimal(value=1.5)" = 1
w = typing.cast("Decimal(3.5)", 1)
v: typing.Literal["Decimal(1.5)"] = 1
print("Decimal(1.5)")
