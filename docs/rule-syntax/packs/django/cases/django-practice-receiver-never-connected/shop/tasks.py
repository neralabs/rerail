from django.core.signals import request_started
from django.dispatch import receiver


@receiver(request_started)
def celery_autodiscovers_tasks(sender, **kwargs):
    pass
