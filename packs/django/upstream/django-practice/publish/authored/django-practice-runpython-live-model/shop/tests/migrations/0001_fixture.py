from django.db import migrations

from shop.helpers import recount_books


def forwards(apps, schema_editor):
    recount_books()


class Migration(migrations.Migration):
    dependencies = []

    operations = [migrations.RunPython(forwards, migrations.RunPython.noop)]
