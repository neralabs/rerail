from django.apps import apps as registry
from django.db import migrations
from django.db.models.signals import post_migrate

from shop.helpers import recount_books


# Runs after `migrate` finishes, on the full schema.
def recount_after(sender=None, **kwargs):
    recount_books()


def forwards(apps, schema_editor):
    post_migrate.connect(recount_after, weak=False, sender=registry.get_app_config("shop"))


class Migration(migrations.Migration):
    dependencies = [("shop", "0010_shelves")]

    operations = [migrations.RunPython(forwards, migrations.RunPython.noop)]
