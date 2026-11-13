from django.conf import settings as django_settings
import django.conf


def toggle():
    django_settings.MAINTENANCE = True  # caught
    django.conf.settings.MAINTENANCE = False  # caught
