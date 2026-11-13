from django.core.signals import request_finished
import django.dispatch


@django.dispatch.receiver(request_finished)  # caught
def only_tests_import_this(sender, **kwargs):
    pass


def plain():
    pass
