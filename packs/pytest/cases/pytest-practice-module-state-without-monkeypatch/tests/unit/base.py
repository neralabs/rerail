from unittest import TestCase

from shop import registry


# A base class whose tearDown restores the timeout for every test of its subclasses; the restore is no leak itself.
class TimeoutCase(TestCase):
    def tearDown(self):
        registry.TIMEOUT = 30
        super().tearDown()
