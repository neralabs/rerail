from django.core.signals import request_started
from django.dispatch import receiver


@receiver(request_started)
def loaded_by_pattern(sender, **kwargs):
    pass
