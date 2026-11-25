import contextlib
import os


# Called only from pytest_sessionstart: session-wide set-up.
def load_test_env():
    os.environ["SHOP_DB"] = "sqlite://"


# Called from a test: the value leaks out of it.
def login_as(user):
    os.environ["SHOP_USER"] = user  # caught


# A context manager that puts the environment back in a finally.
@contextlib.contextmanager
def env(**values):
    saved = os.environ.copy()
    os.environ.update(values)
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(saved)


# A context manager that restores each variable it set, by key, in a loop.
@contextlib.contextmanager
def env_vars(**values):
    old = {key: os.environ.get(key) for key in values}
    os.environ.update(values)
    try:
        yield
    finally:
        for key, value in old.items():
            if value is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = value


# Called by a test that undoes it in its own finally.
def set_region(region):
    os.environ["SHOP_REGION"] = region


# Builds a copy for a subprocess: os.environ itself is not changed.
def child_env(**values):
    environment = os.environ.copy()
    environment.update(values)
    environment["PYTHONUNBUFFERED"] = "1"
    return environment
