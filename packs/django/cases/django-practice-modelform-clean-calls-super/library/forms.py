from django import forms
from django.contrib.auth.forms import UserCreationForm
from django.core.exceptions import ValidationError

from .base import BookFormSetBase, ChapterInlineBase, PlainForm, TrackedModelForm
from .models import Author, Book


class BookForm(TrackedModelForm):
    class Meta:
        model = Book
        fields = ["author", "title"]

    def clean(self):  # caught
        if self.cleaned_data.get("title") == "x":
            raise ValidationError("no")
        return self.cleaned_data


class AuthorForm(forms.ModelForm):
    class Meta:
        model = Author
        fields = ["name"]

    def clean(self):
        data = super().clean()
        data["name"] = data["name"].strip()
        return data


class OldStyleAuthorForm(forms.ModelForm):
    class Meta:
        model = Author
        fields = ["name"]

    def clean(self):
        cleaned_data = super(OldStyleAuthorForm, self).clean()
        return cleaned_data


class ExplicitBaseForm(TrackedModelForm):
    class Meta:
        model = Author
        fields = ["name"]

    def clean(self):
        forms.ModelForm.clean(self)
        return self.cleaned_data


class BookFormSet(BookFormSetBase):
    def clean(self):  # caught
        titles = [f.cleaned_data.get("title") for f in self.forms]
        if len(titles) != len(set(titles)):
            raise ValidationError("duplicate titles")


class GoodBookFormSet(BookFormSetBase):
    def clean(self):
        super().clean()
        if not self.forms:
            raise ValidationError("at least one")


class UniqueBookFormSet(BookFormSetBase):
    # Does by hand all that BaseModelFormSet.clean() does: the uniqueness check runs.
    def clean(self):
        self.validate_unique()
        if not self.forms:
            raise ValidationError("at least one")


class LooseUniqueForm(forms.ModelForm):
    class Meta:
        model = Author
        fields = ["name"]

    # In a form, validate_unique() alone still skips the constraint checks that ModelForm.clean() turns on.
    def clean(self):  # caught
        self.validate_unique()
        return self.cleaned_data


class ChapterFormSet(ChapterInlineBase):
    def clean(self):  # caught
        for form in self.forms:
            form.instance.clean()


class SignupForm(UserCreationForm):
    def clean(self):  # caught
        return self.cleaned_data


class SearchForm(PlainForm):
    def clean(self):
        return self.cleaned_data


class ContactForm(forms.Form):
    def clean(self):
        return self.cleaned_data


class NestedHelperForm(forms.ModelForm):
    class Meta:
        model = Author
        fields = ["name"]

    def clean(self):
        def check():
            return True

        check()
        return super().clean()

    def clean_name(self):
        return self.cleaned_data["name"]


from i18nfield.forms import I18nModelForm  # noqa: E402


class EventForm(I18nModelForm):
    def clean(self):  # caught
        return self.cleaned_data


from django.contrib.admin.options import BaseInlineFormSet as AdminInlineFormSet  # noqa: E402


class ReportFormSet(AdminInlineFormSet):
    def clean(self):  # caught
        if not self.forms:
            raise ValidationError("at least one")
