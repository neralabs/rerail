import os


def env_bool(name):
    return os.environ.get(name) == "1"
