import os
from unittest import mock

import django.core.mail as mail
import pytest
from django.conf import settings
from flask import g

import shop.registry
from shop import registry
from shop.models import Order
from shop.registry import HANDLERS, PLUGINS, Locks, config
from tests.helpers import FakeClock, use_short_timeout

# Module-level code of a test module runs at import, before its tests.
registry.HOOKS["import"].append(print)


def test_module_attr():
    registry.DEFAULT_FLOW = ("confirm",)  # caught


def test_module_attr_dotted():
    shop.registry.TIMEOUT = 1  # caught


def test_augmented():
    registry.TIMEOUT += 5  # caught


def test_delete_after_set():
    registry.TIMEOUT = 5  # caught
    del registry.TIMEOUT


def test_setattr():
    setattr(registry, "TIMEOUT", 2)  # caught


def test_class_attr():
    Locks.timeframe = "5m"  # caught


def test_class_attr_through_module():
    registry.Locks.timeframe = "1d"  # caught


def test_method_replaced():
    Locks.acquire = lambda self: False  # caught


def test_dict_item():
    registry.HANDLERS["refund"] = print  # caught


def test_imported_dict_item():
    HANDLERS["void"] = print  # caught


def test_list_mutator():
    PLUGINS.append("x")  # caught


def test_dict_pop_without_default():
    registry.HANDLERS.pop("refund")  # caught


def test_class_container():
    Locks.seen.append("a")  # caught


def test_module_instance():
    config.debug = True  # caught


def test_helper_class():
    FakeClock.now = 10  # caught


class Recorder:
    events = []


def test_shared_test_class():
    Recorder.events = []  # caught


# Restored by hand at the end: a failing assert skips the restore. Only the first write is reported.
def test_restore_by_hand():
    orig = registry.DEFAULT_FLOW
    registry.DEFAULT_FLOW = ()  # caught
    assert orig
    registry.DEFAULT_FLOW = orig


class TestGrouped:
    def test_in_class(self):
        registry.PLUGINS.insert(0, "first")  # caught


# ---- left alone ----------------------------------------------------------------------------------------------------


def test_monkeypatch(monkeypatch):
    monkeypatch.setattr(registry, "TIMEOUT", 1)
    monkeypatch.setitem(registry.HANDLERS, "x", print)


def test_patch_object():
    with mock.patch.object(Locks, "timeframe", "5m"):
        assert Locks.timeframe == "5m"


def test_write_inside_patch():
    with mock.patch.object(registry, "TIMEOUT", 1):
        registry.TIMEOUT = 3


@mock.patch.dict(registry.HANDLERS, {"a": print})
def test_patch_dict_decorator():
    registry.HANDLERS["b"] = print


def test_patch_string(monkeypatch):
    monkeypatch.setattr("shop.registry.DEFAULT_FLOW", ())
    registry.DEFAULT_FLOW = ("x",)


def test_finally():
    old = registry.TIMEOUT
    registry.TIMEOUT = 1
    try:
        assert registry.TIMEOUT == 1
    finally:
        registry.TIMEOUT = old


def test_outside_libraries():
    mail.outbox = []
    g.user = None
    os.environ["X"] = "1"
    settings.DEBUG = True


def test_instances():
    order = Order()
    order.status = "paid"
    Order.objects.update(status="x")


def test_reads():
    assert registry.TIMEOUT == 30
    assert HANDLERS.get("refund") is None


def test_local_class():
    class Fake:
        calls = 0

    Fake.calls = 1


class Spy:
    calls = []


# A class of this test module that no other module uses: its state is this module's business.
def test_module_local_class():
    Spy.calls = []
    Spy.calls.append(1)


def test_local_rebinding():
    flow = list(registry.DEFAULT_FLOW)
    flow.append("x")
    handlers = {}
    handlers["a"] = print


def test_helper_called():
    use_short_timeout()


# A reset as the only change of the target: emptying the registry before the test.
def test_dict_clear():
    registry.HANDLERS.clear()


def test_with_fixtures(fast_timeout, module_handlers):
    module_handlers["z"] = print
