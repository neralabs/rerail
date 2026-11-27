from django.db import models, transaction
from django.utils.text import slugify

from .base import Stamped


def add_to_update_fields(kwargs, name):
    if kwargs.get("update_fields") is not None:
        kwargs["update_fields"] = {name, *kwargs["update_fields"]}
    return kwargs


class Post(models.Model):
    title = models.CharField(max_length=200)
    slug = models.SlugField()
    words = models.IntegerField(default=0)
    created = models.DateTimeField(null=True)

    def save(self, *args, **kwargs):  # caught
        self.slug = slugify(self.title)
        self.words += 1
        self._cache = None  # not a field: not named
        super().save(*args, **kwargs)


class Page(models.Model):
    title = models.CharField(max_length=200)
    slug = models.SlugField()

    def save(self, **kwargs):  # caught
        if self.title:
            self.slug = slugify(self.title)
        return super(Page, self).save(**kwargs)


class Note(Stamped):
    body = models.TextField()

    def save(self, *args, **kwargs):  # caught
        self.touched_by = "system"  # a field of the base model in base.py
        super().save(*args, **kwargs)


class Retry(models.Model):
    primary = models.BooleanField(default=False)

    # Several forwarding calls: one report, at the method.
    def save(self, *args, **kwargs):  # caught
        self.primary = True
        try:
            with transaction.atomic():
                super().save(*args, **kwargs)
        except Exception:
            self.primary = False
            super().save(*args, **kwargs)


class Handled(models.Model):
    title = models.CharField(max_length=200)
    slug = models.SlugField()

    def save(self, *args, **kwargs):
        self.slug = slugify(self.title)
        update_fields = kwargs.get("update_fields")
        if update_fields is not None:
            kwargs["update_fields"] = {"slug", *update_fields}
        super().save(*args, **kwargs)


class HandledParam(models.Model):
    slug = models.SlugField()

    def save(self, *args, update_fields=None, **kwargs):
        self.slug = "x"
        super().save(*args, update_fields=update_fields, **kwargs)


class HandledByHelper(models.Model):
    slug = models.SlugField()

    # Rebinding kwargs through a helper may add to update_fields: left alone.
    def save(self, **kwargs):
        self.slug = "x"
        kwargs = add_to_update_fields(kwargs, "slug")
        super().save(**kwargs)


class InsertOnly(models.Model):
    created = models.DateTimeField(null=True)
    code = models.CharField(max_length=10)
    owner = models.CharField(max_length=10)
    rank = models.IntegerField(default=0)

    def save(self, *args, **kwargs):
        # An insert writes every field: update_fields does not apply.
        if not self.pk:
            self.created = None
        if self._state.adding:
            self.code = "new"
        if self.pk is None and not self.owner:
            self.owner = "me"
        is_new = self.pk is None
        if is_new:
            self.rank = 1
        super().save(*args, **kwargs)


class InsertOnlyCall(models.Model):
    rank = models.IntegerField(default=0)

    def save(self, *args, **kwargs):
        if self._state.adding:
            self.rank = 1
            super().save(*args, **kwargs)
        else:
            super().save(*args, **kwargs)


class InsertOnlyMore(models.Model):
    key = models.CharField(max_length=10)
    data = models.TextField()

    @property
    def present_in_database(self):
        return not self._state.adding

    def save(self, *args, **kwargs):
        if not self.present_in_database and not self.key:
            self.key = "k"
        if kwargs.get("force_insert") and not self.data:
            self.data = "{}"
        super().save(*args, **kwargs)


class InsertOnlyByKey(models.Model):
    key = models.CharField(max_length=10)
    airflow = models.CharField(max_length=10)

    def save(self, *args, **kwargs):
        is_new = not bool(self.pk)
        if is_new and not self.airflow:
            self.airflow = "front-to-rear"
        if not self.pk or kwargs.get("force_insert", False):
            self.key = "new"
        super().save(*args, **kwargs)


class NewOrChanged(models.Model):
    slug = models.SlugField()

    # `or` with another test runs on updates too.
    def save(self, *args, **kwargs):  # caught
        if not self.pk or self.slug == "":
            self.slug = "x"
        super().save(*args, **kwargs)


class AfterSave(models.Model):
    slug = models.SlugField()

    def save(self, *args, **kwargs):
        super().save(*args, **kwargs)
        # Set after the write, for the next save: not this rule's concern.
        self.slug = "later"


class NoForward(models.Model):
    slug = models.SlugField()

    def save(self):
        # Takes no update_fields at all: nothing to forward.
        self.slug = "x"
        super().save()


class NotAModel:
    slug = None

    def save(self, *args, **kwargs):
        self.slug = "x"
        super().save(*args, **kwargs)


class Comment(models.Model):
    post = models.ForeignKey(Post, on_delete=models.CASCADE)

    def save(self, *args, **kwargs):  # caught
        # `post_id` is the foreign key's column (its attname).
        self.post_id = self.post_id or Post.objects.first().pk
        super().save(*args, **kwargs)
