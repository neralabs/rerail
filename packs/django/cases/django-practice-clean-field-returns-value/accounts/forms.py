import re

from django import forms
from django.core.exceptions import ValidationError

from .base import BaseSignupForm, Helper, StyledForm, StyledModelForm
from .models import Profile, User


class ContactForm(StyledForm):
    email = forms.EmailField()
    name = forms.CharField()
    phone = forms.CharField()
    code = forms.CharField()
    note = forms.CharField()
    tags = forms.CharField()
    city = forms.CharField()
    zip = forms.CharField()
    age = forms.IntegerField()
    nick = forms.CharField()
    title = forms.CharField()
    slug = forms.CharField()

    def clean_email(self):  # caught
        if User.objects.filter(email=self.cleaned_data["email"]).exists():
            raise ValidationError("taken")

    def clean_name(self):
        name = self.cleaned_data["name"]
        if not name:
            return  # caught
        return name.strip()

    def clean_phone(self):  # caught
        phone = self.cleaned_data["phone"]
        if phone.startswith("+"):
            return phone
        elif phone.startswith("00"):
            return "+" + phone[2:]

    def clean_code(self):
        code = self.cleaned_data["code"]
        if not code.isdigit():
            raise ValidationError("digits only")
        return code

    def clean_note(self):
        note = self.cleaned_data["note"]
        if note:
            return note.strip()
        else:
            raise ValidationError("required")

    def clean_tags(self):
        try:
            return [t.strip() for t in self.cleaned_data["tags"].split(",")]
        except AttributeError:
            raise ValidationError("bad tags")

    def clean_city(self):  # caught
        try:
            return self.cleaned_data["city"].title()
        except AttributeError:
            self.add_error("city", "bad city")

    def clean_zip(self):
        value = self.cleaned_data["zip"]
        while True:
            if re.match(r"\d{5}", value):
                return value
            value = "0" + value

    def clean_age(self):
        return None

    def clean_nick(self):
        raise NotImplementedError

    def clean_title(self, value):
        value.strip()

    def clean_slug(self):  # caught
        slug = self.cleaned_data["slug"]
        for ch in slug:
            if not ch.isalnum():
                raise ValidationError("bad slug")
            return slug


class ProfileForm(StyledModelForm):
    class Meta:
        model = Profile
        fields = ["bio", "website"]

    def clean_bio(self):  # caught
        self.cleaned_data["bio"] = self.cleaned_data["bio"].strip()

    def clean_website(self):
        with open("/dev/null"):
            return self.cleaned_data["website"]

    def clean(self):
        cleaned = super().clean()
        cleaned["bio"] = cleaned["bio"] or ""


class SignupForm(BaseSignupForm):
    def clean_username(self):  # caught
        username = self.cleaned_data["username"]
        if username.lower() in {"admin", "root"}:
            raise ValidationError("reserved")


class NotAForm:
    def clean_email(self):
        self.email = self.email.lower()


class StillNotAForm(Helper):
    def clean_value(self):
        pass


class PostForm(forms.Form):
    body = forms.CharField()

    @property
    def attachments_limit(self):
        return 3

    def clean_attachments_limit(self):
        if len(self.data.getlist("files")) > self.attachments_limit:
            raise ValidationError("too many")

    def clean_extra(self):
        if self.data.get("extra"):
            raise ValidationError("no extras")

    def clean(self):
        cleaned = super().clean()
        self.clean_extra()
        return cleaned


from .base import PrevalidatedForm  # noqa: E402


class ImportForm(PrevalidatedForm):
    repo = forms.CharField()

    def clean_prevalidation(self):
        if not self.user.is_staff:
            raise ValidationError("staff only")

    def clean_repo(self):  # caught
        if "github" not in self.cleaned_data["repo"]:
            raise ValidationError("GitHub only")


class MatchForm(forms.Form):
    kind = forms.CharField()
    code = forms.CharField()
    name = forms.CharField()
    note = forms.CharField()
    word = forms.CharField()
    flag = forms.CharField()
    text = forms.CharField()

    def clean_kind(self):  # caught
        # No case catches every value: neither "a" nor "b" falls off the end.
        match self.cleaned_data["kind"]:
            case "a":
                return "A"
            case "b":
                return "B"

    def clean_code(self):  # caught
        # The catch-all cases have guards.
        match self.cleaned_data["code"]:
            case c if c.isdigit():
                return c
            case _ if False:
                raise ValidationError("x")

    def clean_name(self):
        match self.cleaned_data["name"]:
            case "":
                raise ValidationError("empty")
            case other:
                return other

    def clean_flag(self):  # caught
        # `case None:` is a literal pattern, not a capture.
        match self.cleaned_data["flag"]:
            case None:
                return ""

    def clean_note(self):
        # No break: the else always runs, and returns.
        for ch in self.cleaned_data["note"]:
            if ch == "!":
                raise ValidationError("no bangs")
        else:
            return self.cleaned_data["note"]

    def clean_word(self):  # caught
        # A break skips the else and falls off the end.
        for ch in self.cleaned_data["word"]:
            if ch == " ":
                break
        else:
            return self.cleaned_data["word"]

    def clean_text(self):  # caught
        # The else does not return.
        while self.cleaned_data["text"].startswith(" "):
            raise ValidationError("leading space")
        else:
            self.cleaned_data["text"].strip()
