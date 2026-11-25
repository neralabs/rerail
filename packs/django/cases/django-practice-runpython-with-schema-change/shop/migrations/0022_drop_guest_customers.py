from django.db import migrations, models


def drop_guest_customers(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.filter(name="guest").delete()


class Migration(migrations.Migration):
    dependencies = [("shop", "0021_drop_unnamed_customers")]

    # The same deletes with nothing that runs the queued checks first.
    operations = [
        migrations.RunPython(drop_guest_customers, migrations.RunPython.noop),  # caught
        migrations.AlterField(model_name="customer", name="name", field=models.CharField(max_length=200)),
    ]
