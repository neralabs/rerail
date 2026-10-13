def f():
    try:
        return 1
    finally:
        return 2

def f():
    try:
        return 1
    finally:
        return 2  # noqa

def f():
    try:
        return 1
    finally:
        return 2  # noqa: SIM107

def f():
    try:
        return 1
    finally:
        return 2  # NOQA: SIM107

def f():
    try:
        return 1
    finally:
        return 2  # noqa: E501
