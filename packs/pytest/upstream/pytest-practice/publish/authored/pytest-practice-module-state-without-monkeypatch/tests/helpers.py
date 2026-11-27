from shop import registry


class FakeClock:
    now = 0


# A helper a test calls: its write leaks out of that test.
def use_short_timeout():
    registry.TIMEOUT = 1  # caught


# A helper no test calls: not run inside a test.
def use_long_timeout():
    registry.TIMEOUT = 100
