from django.db import migrations


def fill(apps, schema_editor):
    from shop.models import Book
    Book.objects.update(title="")


class Migration(migrations.Migration):
    dependencies = [("shop", "0001_initial")]
    operations = [migrations.RunPython(fill)]
