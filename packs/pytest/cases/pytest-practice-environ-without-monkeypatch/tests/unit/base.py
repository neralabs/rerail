import os
import unittest


# A base class whose tearDown puts the variable back for every test of its subclasses. The undo itself is not a leak.
class TokenCase(unittest.TestCase):
    def tearDown(self):
        os.environ.pop("SHOP_TOKEN", None)
        super().tearDown()
