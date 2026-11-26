import myproject.settings  # caught
import myproject.settings as project_settings  # caught
from myproject import settings  # caught
from myproject.settings import base  # caught
import shop.settings


def page_size():
    return (myproject.settings.DEBUG, project_settings.DEBUG, settings.DEBUG, base.DEBUG, shop.settings.PAGE_SIZE)
