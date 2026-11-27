import os

import pytest


# An autouse fixture that saves the whole environment and puts it back after every test under this directory.
@pytest.fixture(autouse=True)
def restore_environment():
    saved = dict(os.environ)
    yield
    os.environ.clear()
    os.environ.update(saved)
