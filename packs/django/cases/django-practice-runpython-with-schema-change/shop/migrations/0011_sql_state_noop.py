from django.db import migrations, models


def unfill_codes(apps, schema_editor):
    Order = apps.get_model("shop", "Order")
    Order.objects.update(code=None)


def set_labels(apps, schema_editor):
    Tag = apps.get_model("shop", "Tag")
    Tag.objects.update(label="x")


class Migration(migrations.Migration):
    dependencies = [("shop", "0010_coupon")]

    operations = [
        # RunSQL is not RunPython.
        migrations.RunSQL("UPDATE shop_order SET code = 'x'", migrations.RunSQL.noop),
        migrations.AlterField(model_name="order", name="code", field=models.CharField(max_length=12)),
        # Only the reverse code writes.
        migrations.RunPython(migrations.RunPython.noop, unfill_codes),
        migrations.AlterField(model_name="order", name="code", field=models.CharField(max_length=14)),
        # The AlterField changes Django's state only.
        migrations.RunPython(set_labels, migrations.RunPython.noop),
        migrations.SeparateDatabaseAndState(
            state_operations=[
                migrations.AlterField(model_name="tag", name="label", field=models.CharField(max_length=60)),
            ],
        ),
    ]
