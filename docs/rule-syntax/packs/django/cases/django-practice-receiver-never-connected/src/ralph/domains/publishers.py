from django.conf import settings
from django.core.signals import request_started
from django.dispatch import receiver

if settings.DEBUG:
    @receiver(request_started)
    def listed_by_an_app_config_in_apps_py(sender, **kwargs):
        pass
