from django.db import migrations

from shop import helpers
from shop.helpers import due_in, due_in_three


def forwards(apps, schema_editor):
    Book = apps.get_model("shop", "Book")
    for book in Book.objects.all():
        # per_day given by position, through a helper, a module attribute and a direct call: no Shelf query.
        first = due_in_three(len(book.title))
        second = helpers.due_in(1, 4)
        third = due_in(2, 5)
        book.slug = str(first + second + third)
        book.save()


def backwards(apps, schema_editor):
    Book = apps.get_model("shop", "Book")
    for book in Book.objects.all():
        book.slug = str(due_in(len(book.title), None))  # caught
        book.save()


class Migration(migrations.Migration):
    dependencies = [("shop", "0011_after_migrate")]

    operations = [migrations.RunPython(forwards, backwards)]
