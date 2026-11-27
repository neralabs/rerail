from django.db import migrations

from shop import helpers
from shop.jobs import run_job


def forwards(apps, schema_editor):
    run_job(helpers.make_default_shelf)  # caught


def backwards(apps, schema_editor):
    run_job(lambda: helpers.count_via_default_manager())  # caught


class Migration(migrations.Migration):
    dependencies = [("shop", "0003_local_import")]

    operations = [migrations.RunPython(forwards, backwards)]
