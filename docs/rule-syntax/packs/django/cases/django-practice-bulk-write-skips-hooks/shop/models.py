from django.db import models
from django.db.models import F
from django.utils.text import slugify


class Stamped(models.Model):
    updated = models.DateTimeField(null=True)

    class Meta:
        abstract = True

    def save(self, *args, **kwargs):
        self.updated = None
        super().save(*args, **kwargs)


class Book(models.Model):
    title = models.CharField(max_length=100)
    slug = models.SlugField()
    views = models.IntegerField(default=0)

    def save(self, *args, **kwargs):
        self.slug = slugify(self.title)
        super().save(*args, **kwargs)

    def bump(self):
        Book.objects.filter(pk=self.pk).update(views=F("views") + 1)  # caught


class Chapter(Stamped):
    title = models.CharField(max_length=100)


class Note(models.Model):
    text = models.TextField()

    def save(self, *args, **kwargs):
        """Just super."""
        return super().save(*args, **kwargs)

    def delete(self, *args, **kwargs):
        super(Note, self).delete(*args, **kwargs)


class Counter(models.Model):
    n = models.IntegerField(default=0)

    def save(self, *args, **kwargs):
        super().save(*args, **kwargs)
        Counter.objects.filter(pk=self.pk).update(n=F("n") + 1)

    def delete(self, *args, **kwargs):
        Counter.objects.filter(pk=self.pk).delete()


class Archive(models.Model):
    name = models.CharField(max_length=10)

    def delete(self, *args, **kwargs):
        self.name = "gone"
        self.save()


class Tag(models.Model):
    name = models.CharField(max_length=10)


class Label(models.Model):
    name = models.CharField(max_length=10)


class Plain(models.Model):
    name = models.CharField(max_length=10)
