import os

import pytest

from tests.helpers import env_vars, set_region


# A function-scoped fixture whose teardown puts the variable back after the test that requests it.
@pytest.fixture
def restore_locale():
    old = os.environ.get("LANG")
    yield
    if old is None:
        os.environ.pop("LANG", None)
    else:
        os.environ["LANG"] = old


def test_restored_by_requested_fixture(restore_locale):
    os.environ["LANG"] = "de_DE.UTF-8"


# A context manager helper that restores the variables it set, by key, in a loop.
def test_context_manager_loop_restore():
    with env_vars(SHOP_A="1", SHOP_B="2"):
        assert os.environ["SHOP_A"] == "1"


# A helper whose change the calling test undoes in its finally.
def test_helper_undone_by_caller():
    set_region("eu")
    try:
        assert os.environ["SHOP_REGION"] == "eu"
    finally:
        os.environ.pop("SHOP_REGION", None)


# A helper the undo code calls.
def _restore_home(old):
    os.environ["HOME"] = old


def test_undo_through_helper():
    old = os.environ["HOME"]
    os.environ["HOME"] = "/tmp/home"
    try:
        assert os.path.isdir("/tmp")
    finally:
        _restore_home(old)


# A context-manager class: __exit__ puts back what __init__ set.
class TempVar:
    def __init__(self, name, value):
        self.name = name
        self.old = os.environ.get(name)
        os.environ["SHOP_TEMP"] = value

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        os.environ.pop("SHOP_TEMP", None)


def test_context_manager_class():
    with TempVar("SHOP_TEMP", "1"):
        assert os.environ["SHOP_TEMP"] == "1"


# Augmented assignment of the mapping itself.
def test_merge_operator():
    os.environ |= {"SHOP_MERGED": "1"}  # caught


# A helper of the test module that no test calls: not run inside a test.
def unused_helper():
    os.environ["SHOP_UNUSED"] = "1"


# A test-module helper called by a test, through self.
class TestThroughMethod:
    def _configure(self):
        os.environ["SHOP_METHOD"] = "1"  # caught

    def test_configured(self):
        self._configure()


# A fixture whose yield expression calls a set-up helper: the helper runs before the yield, nothing undoes it.
def _start_service():
    os.environ["SHOP_SERVICE"] = "1"  # caught
    return "service"


@pytest.fixture
def service():
    yield _start_service()


def test_service(service):
    assert service
