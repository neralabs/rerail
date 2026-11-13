from django.core.signals import request_started
from django.dispatch import receiver


@receiver(request_started)
def admin_is_autodiscovered(sender, **kwargs):
    pass
