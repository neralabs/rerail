assert (x := 1) == 1  
print(x)

assert (x := 1) == 1    # noqa
print(x)

assert (x := 1) == 1    # noqa: RUF018
print(x)

assert (x := 1) == 1    # NOQA: RUF018
print(x)

assert (x := 1) == 1    # noqa: E501
print(x)
