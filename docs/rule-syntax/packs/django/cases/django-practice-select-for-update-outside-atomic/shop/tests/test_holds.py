from django.test import TestCase

from shop.models import Hold
from shop.services import lock_safe
from shop.tasks import expire


class HoldTests(TestCase):
    def test_lock(self):
        # TestCase's transaction hides the missing atomic(); test code is not reported.
        Hold.objects.select_for_update().get(pk=1)
        lock_safe(1)

    def test_expire(self):
        # A test calling the task directly does not make it covered: production runs it from the queue.
        expire(1)
        expire.delay(1)
