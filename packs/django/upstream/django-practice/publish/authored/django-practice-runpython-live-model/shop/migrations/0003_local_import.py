from django.db import migrations


def forwards(apps, schema_editor):
    from shop.helpers import refresh_counts

    refresh_counts()  # caught


class Migration(migrations.Migration):
    dependencies = [("shop", "0002_recount")]

    operations = [migrations.RunPython(forwards, migrations.RunPython.noop)]
