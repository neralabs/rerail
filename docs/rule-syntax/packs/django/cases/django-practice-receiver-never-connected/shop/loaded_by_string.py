from django.core.signals import request_started
from django.dispatch import receiver


@receiver(request_started)
def named_in_a_string(sender, **kwargs):
    pass
