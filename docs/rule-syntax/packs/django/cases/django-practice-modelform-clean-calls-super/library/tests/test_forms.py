from django import forms

from ..models import Author


class BrokenForm(forms.ModelForm):
    class Meta:
        model = Author
        fields = ["name"]

    def clean(self):
        return self.cleaned_data
