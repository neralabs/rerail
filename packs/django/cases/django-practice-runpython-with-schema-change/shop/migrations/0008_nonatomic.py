from django.db import migrations, models


def fill_codes(apps, schema_editor):
    Order = apps.get_model("shop", "Order")
    Order.objects.filter(code=None).update(code="legacy")


class Migration(migrations.Migration):
    # Each operation commits on its own.
    atomic = False

    dependencies = [("shop", "0007_tag_rename")]

    operations = [
        migrations.AddField(model_name="order", name="code", field=models.CharField(max_length=10, null=True)),
        migrations.RunPython(fill_codes, migrations.RunPython.noop),
        migrations.AlterField(model_name="order", name="code", field=models.CharField(max_length=10)),
    ]
