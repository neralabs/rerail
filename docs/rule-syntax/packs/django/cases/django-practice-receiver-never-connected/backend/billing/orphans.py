from django.core.signals import request_started
from django.dispatch import receiver


@receiver(request_started)  # caught
def orphan(sender, **kwargs):
    pass
