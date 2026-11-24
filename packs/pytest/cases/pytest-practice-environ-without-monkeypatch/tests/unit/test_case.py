import os
import unittest
from unittest import mock


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


class AddCleanup(unittest.TestCase):
    def test_cleanup(self):
        os.environ["SHOP_LOG"] = "debug"
        self.addCleanup(os.environ.pop, "SHOP_LOG")

    def test_cleanup_lambda(self):
        os.environ["SHOP_LEVEL"] = "1"
        self.addCleanup(lambda: os.environ.pop("SHOP_LEVEL"))


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
