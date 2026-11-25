from django.db import migrations, models


def drop_blank_customers(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.filter(email="").delete()


class Migration(migrations.Migration):
    dependencies = [("shop", "0019_customer_status")]

    # SET CONSTRAINTS ALL IMMEDIATE runs the checks the deletes queued before the table is altered.
    operations = [
        migrations.RunPython(drop_blank_customers, migrations.RunPython.noop),
        migrations.RunSQL("SET CONSTRAINTS ALL IMMEDIATE", migrations.RunSQL.noop),
        migrations.AlterField(model_name="customer", name="name", field=models.CharField(max_length=120)),
    ]
