fs = []
for x in range(3):
    fs.append(lambda: x)

fs = []
for x in range(3):
    fs.append(lambda: x)  # noqa

fs = []
for x in range(3):
    fs.append(lambda: x)  # noqa: B023

fs = []
for x in range(3):
    fs.append(lambda: x)  # NOQA: B023

fs = []
for x in range(3):
    fs.append(lambda: x)  # noqa: E501
