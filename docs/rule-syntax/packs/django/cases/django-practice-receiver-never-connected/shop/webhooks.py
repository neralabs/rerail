from django.core.signals import request_started
from django.dispatch import receiver


@receiver(request_started)
def named_by_an_app_config(sender, **kwargs):
    pass
