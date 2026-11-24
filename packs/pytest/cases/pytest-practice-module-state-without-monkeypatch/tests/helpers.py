from shop import registry


class FakeClock:
    now = 0


# A helper, not a test, fixture or set-up hook: not read (a gap).
def use_short_timeout():
    registry.TIMEOUT = 1
