from django.db import migrations, models


def clear_flags(apps, schema_editor):
    for name in ("Order", "Customer"):
        model = apps.get_model("shop", name)
        model.objects.update(flagged=False)


class Migration(migrations.Migration):
    dependencies = [("shop", "0011_sql_state_noop")]

    # The model names come from a loop over literals and are read; an update of a column no key covers queues no check.
    operations = [
        migrations.RunPython(clear_flags, migrations.RunPython.noop),
        migrations.RemoveField(model_name="order", name="flagged"),
    ]
