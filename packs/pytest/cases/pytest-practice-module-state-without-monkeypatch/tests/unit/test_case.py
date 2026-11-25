from unittest import TestCase, mock

from shop import registry
from shop.registry import Locks
from tests.unit.base import TimeoutCase


class Restored(TestCase):
    def setUp(self):
        self.old = registry.TIMEOUT
        registry.TIMEOUT = 1

    def tearDown(self):
        registry.TIMEOUT = self.old

    def test_x(self):
        self.value = 1
        registry.TIMEOUT = 2


class NotRestored(TestCase):
    def setUp(self):
        Locks.timeframe = "5m"  # caught

    def tearDown(self):
        registry.TIMEOUT = 30

    def test_x(self):
        registry.PLUGINS.extend(["a"])  # caught


class ClassLevel(TestCase):
    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        registry.HANDLERS["k"] = print
        cls.flag = True

    @classmethod
    def tearDownClass(cls):
        registry.HANDLERS.pop("k")
        super().tearDownClass()


class Cleanups(TestCase):
    def test_add_cleanup(self):
        self.addCleanup(setattr, registry, "TIMEOUT", registry.TIMEOUT)
        registry.TIMEOUT = 9

    def test_cleanup_lambda(self):
        old = Locks.timeframe
        self.addCleanup(lambda: setattr(Locks, "timeframe", old))
        Locks.timeframe = "1m"

    def test_cleanup_clear(self):
        self.addCleanup(registry.HANDLERS.clear)
        registry.HANDLERS["z"] = print


# A patcher started in setUp and stopped by a cleanup restores the attribute after every test of the class.
class PatchedInSetUp(TestCase):
    def setUp(self):
        patcher = mock.patch.object(registry, "TIMEOUT", 3)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_x(self):
        registry.TIMEOUT = 4


# The base class's tearDown restores the attribute.
class ChildOfBase(TimeoutCase):
    def test_timeout(self):
        registry.TIMEOUT = 1

    def test_other_target(self):
        registry.DEFAULT_FLOW = ("x",)  # caught


# A bound method registered as the cleanup.
class BoundMethodCleanup(TestCase):
    def setUp(self):
        self.old = Locks.timeframe
        Locks.timeframe = "1s"
        self.addCleanup(self._restore)

    def _restore(self):
        Locks.timeframe = self.old
