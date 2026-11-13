from django.conf import settings
from django.conf import global_settings
from rest_framework.settings import api_settings

from myproject.settings import DEBUG  # caught
from myproject.settings import (
    SITE_URL,  # caught
    env_bool,
)
from myproject.settings.local import FEATURE_FLAG  # caught
from config.prod import DEBUG as PROD_DEBUG  # caught
from deploy.config import ALLOWED_HOSTS  # caught
from shop.settings import PAGE_SIZE
from .settings import PAGE_SIZE as SIZE


def view(request):
    from myproject.settings.base import SITE_URL as URL  # caught

    return (settings.DEBUG, global_settings.LANGUAGES, api_settings, DEBUG, SITE_URL, env_bool, FEATURE_FLAG,
            PROD_DEBUG, ALLOWED_HOSTS, PAGE_SIZE, SIZE, URL)
