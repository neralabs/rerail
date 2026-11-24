from django.db import migrations

from shop.helpers import count_via_default_manager


def _report():
    return count_via_default_manager()  # caught


def _unused():
    return count_via_default_manager()


def forwards(apps, schema_editor):
    _report()


class Migration(migrations.Migration):
    dependencies = [("shop", "0005_helper_is_the_code")]

    operations = [migrations.RunPython(code=forwards, reverse_code=migrations.RunPython.noop)]
