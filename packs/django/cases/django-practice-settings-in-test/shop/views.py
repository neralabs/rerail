from django.conf import settings


# Not test code: the django pack's settings-runtime-assignment rule reports this one.
def enable_debug():
    settings.DEBUG = True
