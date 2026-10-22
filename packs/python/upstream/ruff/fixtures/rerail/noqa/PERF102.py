d = {}
for k, v in d.items():
    print(k)

d = {}
for k, v in d.items():  # noqa
    print(k)

d = {}
for k, v in d.items():  # noqa: PERF102
    print(k)

d = {}
for k, v in d.items():  # NOQA: PERF102
    print(k)

d = {}
for k, v in d.items():  # noqa: E501
    print(k)
