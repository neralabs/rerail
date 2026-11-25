from django.db import migrations, models


def clear_order_notes(apps, schema_editor):
    for name in ["Order"]:
        model = apps.get_model("shop", name)
        model.objects.update(note="")


class Migration(migrations.Migration):
    dependencies = [("shop", "0022_drop_guest_customers")]

    # The model name comes from a loop over literal names: Order's whole table is rewritten under the AddField's lock.
    operations = [
        migrations.AddField(model_name="order", name="archived", field=models.BooleanField(default=False)),
        migrations.RunPython(clear_order_notes, migrations.RunPython.noop),  # caught
    ]
