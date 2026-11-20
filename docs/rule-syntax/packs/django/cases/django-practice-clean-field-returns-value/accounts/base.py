from django import forms
from django.contrib.auth.forms import UserCreationForm


class StyledForm(forms.Form):
    """A workspace base: forms in other files derive from it."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        for field in self.fields.values():
            field.widget.attrs["class"] = "input"


class StyledModelForm(forms.ModelForm):
    pass


class BaseSignupForm(UserCreationForm):
    pass


class Helper:
    """Not a form: its clean_ methods are not Django's."""

    def clean_text(self):
        self.text = self.text.strip()


class PrevalidatedForm(forms.Form):
    """Runs clean_prevalidation() itself, before any field: a hook of this base, not a field's."""

    def is_valid(self):
        self.clean_prevalidation()
        return super().is_valid()

    def clean_prevalidation(self):
        pass
