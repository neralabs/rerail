from django.db import migrations

from shop.helpers import due_in, due_in_one, due_in_two


def forwards(apps, schema_editor):
    Book = apps.get_model("shop", "Book")
    for book in Book.objects.all():
        book.slug = str(due_in_one(len(book.title)) + due_in_two(1))
        book.save()


def backwards(apps, schema_editor):
    Book = apps.get_model("shop", "Book")
    for book in Book.objects.all():
        book.slug = str(due_in(len(book.title)))  # caught
        book.save()


class Migration(migrations.Migration):
    dependencies = [("shop", "0008_direct_live_model")]

    operations = [migrations.RunPython(forwards, backwards)]
