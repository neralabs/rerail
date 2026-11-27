from django.db import migrations

from shop.helpers import count_books, fill_slugs, is_done, make_admin
from shop.utils import normalize_url


def forwards(apps, schema_editor):
    Book = apps.get_model("shop", "Book")
    fill_slugs(Book)
    count_books(apps.get_model("shop", "Book"))
    for book in Book.objects.all():
        book.slug = normalize_url(book.slug)
        if is_done(book):
            book.save()
    make_admin()


class Migration(migrations.Migration):
    dependencies = [("shop", "0006_through_local_def")]

    operations = [migrations.RunPython(forwards, migrations.RunPython.noop)]
