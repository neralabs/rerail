from django.db import migrations

from shop.helpers import make_named_shelf, shelf_label


def forwards(apps, schema_editor):
    Book = apps.get_model("shop", "Book")
    for book in Book.objects.all():
        book.slug = shelf_label(book.title)
        book.save()
    make_named_shelf("archive")  # caught


class Migration(migrations.Migration):
    dependencies = [("shop", "0009_due_dates")]

    operations = [migrations.RunPython(forwards, migrations.RunPython.noop)]
