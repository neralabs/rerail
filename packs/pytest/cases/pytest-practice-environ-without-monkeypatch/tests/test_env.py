import os
from os import environ
from unittest import mock

import pytest

from tests.helpers import env, login_as

# Module-level code of a test module runs once when pytest imports it, before the first test: every test sees it.
os.environ["FEATURE_BETA"] = "1"


def test_set_item():
    os.environ["SHOP_CURRENCY"] = "EUR"  # caught
    assert os.environ["SHOP_CURRENCY"] == "EUR"


def test_augmented():
    os.environ["PATH"] += ":/opt/shop/bin"  # caught


def test_del():
    del os.environ["HOME"]  # caught


def test_from_import():
    environ["SHOP_MODE"] = "offline"  # caught


def test_mutators():
    os.environ.pop("SHOP_TOKEN", None)  # caught
    os.environ.setdefault("SHOP_REGION", "eu")  # caught
    os.environ.update(SHOP_LANG="de")  # caught


def test_clear():
    os.environ.clear()  # caught


def test_putenv():
    os.putenv("SHOP_TRACE", "1")  # caught
    os.unsetenv("SHOP_DEBUG")  # caught


# A restore by hand at the end of the test: a failing assert skips it. Reported once, at the first write.
def test_restore_by_hand():
    old = os.environ.get("SHOP_URL")
    os.environ["SHOP_URL"] = "http://test"  # caught
    assert os.environ["SHOP_URL"] == "http://test"
    os.environ["SHOP_URL"] = old


def test_helper_called():
    login_as("alice")


def test_restore_in_finally():
    os.environ["SHOP_SECRET"] = "s3cret"
    try:
        assert os.environ["SHOP_SECRET"]
    finally:
        del os.environ["SHOP_SECRET"]


def test_pop_in_finally():
    os.environ.update({"SHOP_A": "1", "SHOP_B": "2"})
    try:
        assert True
    finally:
        os.environ.pop("SHOP_A")
        os.environ.pop("SHOP_B")


# The finally restores only one of the two variables.
def test_finally_misses_one():
    os.environ.update({"SHOP_C": "1", "SHOP_D": "2"})  # caught
    try:
        assert True
    finally:
        os.environ.pop("SHOP_C")


def test_monkeypatch(monkeypatch):
    monkeypatch.setenv("SHOP_CURRENCY", "EUR")
    monkeypatch.delenv("HOME")


def test_monkeypatch_same_key(monkeypatch):
    monkeypatch.setenv("SHOP_PORT", "80")
    os.environ["SHOP_PORT"] = "8080"


def test_patch_dict_with():
    with mock.patch.dict(os.environ, {"SHOP_CURRENCY": "EUR"}):
        os.environ["SHOP_EXTRA"] = "1"
    with mock.patch.dict("os.environ", clear=True):
        os.environ.update({"ONLY": "1"})


@mock.patch.dict(os.environ, {"SHOP_CURRENCY": "EUR"})
def test_patch_dict_decorator():
    os.environ["SHOP_EXTRA"] = "1"


def test_context_manager_helper():
    with env(SHOP_CURRENCY="EUR"):
        assert os.environ["SHOP_CURRENCY"] == "EUR"


# Reads, and copies of the environment.
def test_reads():
    assert os.environ.get("SHOP_CURRENCY") is None
    assert "HOME" in os.environ
    copy = os.environ.copy()
    copy["SHOP_CURRENCY"] = "EUR"
    copy.pop("HOME", None)


# A WSGI environ dict is not the process environment.
def test_wsgi_environ():
    environ = {"PATH_INFO": "/"}
    environ["REQUEST_METHOD"] = "GET"
    environ.update({"QUERY_STRING": ""})


def test_request_environ(rf):
    request = rf.get("/")
    request.environ["REMOTE_ADDR"] = "10.0.0.1"
    request.META["HTTP_HOST"] = "shop"


# Rebinding os.environ is Ruff's B003.
def test_rebind():
    os.environ = {}


@pytest.fixture
def addcleanup_style(request):
    os.environ["SHOP_CACHE"] = "off"

    def restore():
        del os.environ["SHOP_CACHE"]

    request.addfinalizer(restore)
