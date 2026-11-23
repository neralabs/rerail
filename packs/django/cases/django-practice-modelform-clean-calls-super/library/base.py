from django import forms
from django.forms import BaseInlineFormSet
from django.forms.models import BaseModelFormSet


class TrackedModelForm(forms.ModelForm):
    """A workspace base in another file."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.touched = set()


class BookFormSetBase(BaseModelFormSet):
    pass


class ChapterInlineBase(BaseInlineFormSet):
    pass


class PlainForm(forms.Form):
    pass
