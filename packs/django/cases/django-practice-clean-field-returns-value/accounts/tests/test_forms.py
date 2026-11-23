from django import forms


class FakeForm(forms.Form):
    def clean_email(self):
        pass
