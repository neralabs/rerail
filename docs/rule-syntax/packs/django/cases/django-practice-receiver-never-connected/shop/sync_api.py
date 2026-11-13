from django.core.signals import request_started
from django.dispatch import receiver


def connect_sync(name):
    @receiver(request_started, dispatch_uid=name)
    def nested(sender, **kwargs):
        pass


class Hooks:
    @receiver(request_started)  # caught
    def in_class_body(sender, **kwargs):
        pass
