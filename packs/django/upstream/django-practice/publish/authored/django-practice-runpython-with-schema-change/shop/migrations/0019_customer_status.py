from django.db import migrations, models


def mark_house_account(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.filter(pk=1).update(status="house")


def mark_owner(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    owner = Customer.objects.get(id=2)
    owner.status = "owner"
    owner.save()


def mark_listed(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    for pk in [3, 4, 5]:
        Customer.objects.filter(pk=pk).update(status="listed")


class Migration(migrations.Migration):
    dependencies = [("shop", "0018_tag_product")]

    # One row by primary key holds the lock for milliseconds; the same write in a loop is a backfill.
    operations = [
        migrations.AddField(model_name="customer", name="status", field=models.CharField(max_length=10, null=True)),
        migrations.RunPython(mark_house_account, migrations.RunPython.noop),
        migrations.RunPython(mark_owner, migrations.RunPython.noop),
        migrations.RunPython(mark_listed, migrations.RunPython.noop),  # caught
    ]
