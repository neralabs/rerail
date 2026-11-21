import json

from django.db import models
from django.utils.translation import gettext
from django.utils.translation import gettext as _

# Turned into text right away: the message points at format_lazy / building the text per use.
TITLE = _("Hello %s") % "x"  # caught
WRAPPED = (_("Hello %s")) % "x"  # caught
PREFIXED = "Note: " + _("read me")  # caught
GREETING = f"{_('Welcome')}!"  # caught
LABEL = _("Count: {}").format(3)  # caught
HEAD = _("Library").upper()  # caught
PAYLOAD = json.dumps({"title": _("Title")})  # caught
AS_TEXT = str(_("Plain"))  # caught
ERRORS = {_("missing"): 1}  # caught

# A module-level alias of gettext.
__ = gettext


class Book(models.Model):
    title = models.CharField(__("title"), max_length=80)  # caught


def describe():
    # Runs per call: fine with the alias too.
    return __("book") % {}
