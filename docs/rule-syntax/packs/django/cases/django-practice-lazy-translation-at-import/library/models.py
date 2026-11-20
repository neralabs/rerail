from django.contrib import admin
from django.db import models
from django.utils import translation
from django.utils.translation import gettext as _
from django.utils.translation import gettext_lazy, gettext_noop, ngettext, pgettext
from django.utils.translation import gettext_lazy as _lazy

STATUS_CHOICES = [
    ("draft", _("Draft")),  # caught
    ("published", _lazy("Published")),
]
NOOP = gettext_noop("Marked for extraction only")
HEADING = translation.gettext("Library")  # caught


class Book(models.Model):
    title = models.CharField(_("title"), max_length=80)  # caught
    summary = models.TextField(help_text=pgettext("book", "Short summary"))  # caught
    isbn = models.CharField(gettext_lazy("ISBN"), max_length=13)

    class Meta:
        verbose_name = _("book")  # caught
        verbose_name_plural = gettext_lazy("books")

    def __str__(self):
        # Runs per call, in the request's language.
        return _("Book %(title)s") % {"title": self.title}

    def describe(self, n):
        message = ngettext("%(n)d copy", "%(n)d copies", n)
        return message % {"n": n}

    def heading(self, text=_("Untitled")):  # caught
        return text

    @admin.display(description=_("Title length"))  # caught
    def title_length(self):
        return len(self.title)


def make_label(prefix=_("Item")):  # caught
    labels = [_("one"), _("two")]
    return prefix, labels, (lambda: _("lazy enough"))


def factory():
    class Local(models.Model):
        # Defined when factory() runs, not at import.
        name = models.CharField(_("name"), max_length=10)

    return Local


# A statement of its own only marks the text for makemessages: nothing reads the translation.
_("Scrum")
