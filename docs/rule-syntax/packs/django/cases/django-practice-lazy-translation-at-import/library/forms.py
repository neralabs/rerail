from django import forms
from django.utils.translation import ugettext as _

from .models import Book


class BookForm(forms.ModelForm):
    note = forms.CharField(label=_("Note"), required=False)  # caught

    class Meta:
        model = Book
        fields = ["title"]
        labels = {"title": _("Book title")}  # caught

    def clean_title(self):
        raise forms.ValidationError(_("Bad title"))
