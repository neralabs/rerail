from django.db import migrations

from shop.helpers import recount_books


def forwards(apps, schema_editor):
    recount_books()  # caught


class Migration(migrations.Migration):
    dependencies = [("shop", "0001_initial")]

    operations = [migrations.RunPython(forwards, migrations.RunPython.noop)]
