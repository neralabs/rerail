x = [1, 2]
for i in list(x):
    print(i)

x = [1, 2]
for i in list(x):  # noqa
    print(i)

x = [1, 2]
for i in list(x):  # noqa: PERF101
    print(i)

x = [1, 2]
for i in list(x):  # NOQA: PERF101
    print(i)

x = [1, 2]
for i in list(x):  # noqa: E501
    print(i)
