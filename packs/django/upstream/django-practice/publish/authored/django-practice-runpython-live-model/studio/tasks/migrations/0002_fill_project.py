from django.db import migrations

from tasks.functions import fill_project


def forward(apps, schema_editor):
    fill_project()  # caught


class Migration(migrations.Migration):
    dependencies = []

    operations = [migrations.RunPython(forward, migrations.RunPython.noop)]
