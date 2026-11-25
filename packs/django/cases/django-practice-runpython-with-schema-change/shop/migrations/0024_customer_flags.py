from django.db import migrations, models


def flag_ops_account(apps, schema_editor):
    # email is unique since 0003: one row.
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.filter(email="ops@example.com").update(vip=True)


def flag_first_customer(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    first = Customer.objects.order_by("pk").first()
    first.vip = True
    first.save()


def flag_named(apps, schema_editor):
    # name is not unique: every matching row.
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.filter(name="Ops").update(vip=True)


def flag_one(apps, pk):
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.filter(pk=pk).update(vip=True)


def flag_listed(apps, schema_editor):
    # One row per call, but the calls run in a loop.
    for pk in range(100):
        flag_one(apps, pk)


class Migration(migrations.Migration):
    dependencies = [("shop", "0023_loop_models")]

    operations = [
        migrations.AddField(model_name="customer", name="vip", field=models.BooleanField(default=False)),
        migrations.RunPython(flag_ops_account, migrations.RunPython.noop),
        migrations.RunPython(flag_first_customer, migrations.RunPython.noop),
        migrations.RunPython(flag_named, migrations.RunPython.noop),  # caught
        migrations.RunPython(flag_listed, migrations.RunPython.noop),  # caught
    ]
