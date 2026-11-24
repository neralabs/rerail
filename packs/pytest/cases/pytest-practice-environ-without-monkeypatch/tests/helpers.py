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


# Builds a copy for a subprocess: os.environ itself is not changed.
def child_env(**values):
    environment = os.environ.copy()
    environment.update(values)
    environment["PYTHONUNBUFFERED"] = "1"
    return environment
