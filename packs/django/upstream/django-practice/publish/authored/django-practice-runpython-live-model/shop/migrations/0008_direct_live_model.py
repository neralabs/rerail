from django.db import migrations

from shop.models import Book


# The live model written in the migration itself: django-migration-linter's RUNPYTHON_MODEL_IMPORT reports it.
def forwards(apps, schema_editor):
    Book.objects.filter(slug="").update(slug="untitled")


class Migration(migrations.Migration):
    dependencies = [("shop", "0007_safe_helpers")]

    operations = [migrations.RunPython(forwards, migrations.RunPython.noop)]
