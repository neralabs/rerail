import os
import unittest
from unittest import mock

from tests.unit.base import TokenCase


class RestoredInTearDown(unittest.TestCase):
    def setUp(self):
        self.old = os.environ.get("SHOP_HOST")
        os.environ["SHOP_HOST"] = "localhost"

    def tearDown(self):
        os.environ["SHOP_HOST"] = self.old

    def test_host(self):
        assert os.environ["SHOP_HOST"] == "localhost"


class NoTearDown(unittest.TestCase):
    def setUp(self):
        os.environ["SHOP_PORT"] = "8000"  # caught

    def test_port(self):
        os.environ["SHOP_SCHEME"] = "https"  # caught


class ClassLevelNoTearDown(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        os.environ["SHOP_CLASS"] = "1"  # caught


class AddCleanup(unittest.TestCase):
    def test_cleanup(self):
        os.environ["SHOP_LOG"] = "debug"
        self.addCleanup(os.environ.pop, "SHOP_LOG")

    def test_cleanup_lambda(self):
        os.environ["SHOP_LEVEL"] = "1"
        self.addCleanup(lambda: os.environ.pop("SHOP_LEVEL"))


# A bound method registered as the cleanup.
class BoundMethodCleanup(unittest.TestCase):
    def setUp(self):
        self.old = os.environ.get("SHOP_RESOLVER")
        os.environ["SHOP_RESOLVER"] = "fake"
        self.addCleanup(self._restore_resolver)

    def _restore_resolver(self):
        os.environ.pop("SHOP_RESOLVER", None)


# The base class's tearDown removes the variable after each test.
class ChildOfBase(TokenCase):
    def test_token(self):
        os.environ["SHOP_TOKEN"] = "t0k"


class ChildOfBaseOtherVar(TokenCase):
    def test_other(self):
        os.environ["SHOP_SECRET"] = "s"  # caught


@mock.patch.dict(os.environ, {"SHOP_MODE": "test"})
class PatchedClass(unittest.TestCase):
    def test_mode(self):
        os.environ["SHOP_EXTRA"] = "1"


class StartedPatcher(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch.dict(os.environ, {"SHOP_MODE": "test"})
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_mode(self):
        os.environ["SHOP_EXTRA"] = "1"
