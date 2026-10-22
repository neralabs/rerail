for i in range(3):
    try:
        int("x")
    except ValueError:
        continue

for i in range(3):
    try:
        int("x")
    except ValueError:  # noqa
        continue

for i in range(3):
    try:
        int("x")
    except ValueError:  # noqa: PERF203
        continue

for i in range(3):
    try:
        int("x")
    except ValueError:  # NOQA: PERF203
        continue

for i in range(3):
    try:
        int("x")
    except ValueError:  # noqa: E501
        continue
