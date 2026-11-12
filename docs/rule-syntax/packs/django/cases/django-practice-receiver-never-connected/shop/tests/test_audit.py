from django.core.signals import request_started
from django.dispatch import receiver

from shop import audit  # noqa: F401


@receiver(request_started)
def test_receiver(sender, **kwargs):
    pass
