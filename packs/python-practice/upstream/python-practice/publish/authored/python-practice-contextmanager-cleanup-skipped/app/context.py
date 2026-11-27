import threading

_local = threading.local()


def get_user():
    return getattr(_local, "user", None)


def set_user(user):
    _local.user = user


class Marker:
    def __init__(self, path):
        self.path = path
