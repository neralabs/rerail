from django.apps import AppConfig
from django.utils.translation import gettext as _


class LibraryConfig(AppConfig):
    name = "library"
    verbose_name = _("Library")  # caught
