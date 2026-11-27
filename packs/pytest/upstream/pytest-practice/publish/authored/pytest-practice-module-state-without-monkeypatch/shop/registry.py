from collections import defaultdict

DEFAULT_FLOW = ("cart", "payment", "confirm")
HANDLERS = {}
PLUGINS = []
HOOKS = defaultdict(list)
TIMEOUT = 30


class Locks:
    timeframe = "1h"
    seen = []

    def acquire(self):
        return True


class Config:
    def __init__(self):
        self.debug = False


config = Config()


def reset():
    # Not test code: a module writing its own state.
    HANDLERS.clear()
