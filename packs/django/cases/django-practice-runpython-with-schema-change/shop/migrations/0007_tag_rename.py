from django.db import migrations


def _tag_orders(apps):
    Order = apps.get_model("shop", "Order")
    Tag = apps.get_model("shop", "Tag")
    for order in Order.objects.filter(tag__isnull=True):
        Tag(order=order, name="untagged").save()


def forwards(apps, schema_editor):
    _tag_orders(apps)


class Migration(migrations.Migration):
    dependencies = [("shop", "0006_walk_in_order")]

    # RunPython(atomic=False) does not leave the migration's transaction: only a migration with atomic = False does.
    operations = [
        migrations.RunPython(forwards, migrations.RunPython.noop, atomic=False),  # caught
        migrations.RenameField(model_name="tag", old_name="name", new_name="label"),
    ]
