from django.conf import settings
from django.http import HttpResponse


def enable_debug(request):
    settings.DEBUG = True  # caught
    return HttpResponse("ok")


def add_app(name):
    settings.INSTALLED_APPS += [name]  # caught


def set_cache():
    settings.CACHES["default"] = {"BACKEND": "x"}  # caught
    settings.CACHES["default"]["LOCATION"] = "y"  # caught


def drop_idp(name):
    del settings.SOCIAL_AUTH_IDPS[name]  # caught
    del settings.FEATURE_FLAG  # caught


def by_name(value):
    setattr(settings, "S3_TIMEOUT", value)  # caught
    delattr(settings, "S3_TIMEOUT")  # caught


# Reads are fine.
def read():
    timeout = settings.S3_TIMEOUT
    flags = settings.CACHES["default"]
    return timeout, flags


# A lower-case attribute is not a setting (Django ignores lower-case names in settings files).
def internals():
    settings._wrapped = None


# setattr with a computed name: no setting name to report.
def dynamic(key, value):
    setattr(settings, key, value)


# Module level counts too: it still runs after settings are loaded.
settings.USING_TORNADO = False  # caught
