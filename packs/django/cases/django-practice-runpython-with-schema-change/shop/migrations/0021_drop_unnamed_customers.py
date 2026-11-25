from django.db import migrations, models


def drop_unnamed_customers(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.filter(name="").delete()
    schema_editor.execute("SET CONSTRAINTS ALL IMMEDIATE")


class Migration(migrations.Migration):
    dependencies = [("shop", "0020_drop_blank_customers")]

    # The RunPython runs the queued checks itself before the table is altered.
    operations = [
        migrations.RunPython(drop_unnamed_customers, migrations.RunPython.noop),
        migrations.AlterField(model_name="customer", name="name", field=models.CharField(max_length=150)),
    ]
