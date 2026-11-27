from django.db import migrations, models


def default_tier(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    Customer.objects.filter(tier="").update(tier="basic")


class Migration(migrations.Migration):
    # 0002_order_number is replaced by 0001_squashed_0002_order_number, which defines Customer.tier.
    dependencies = [("shop", "0002_order_number"), ("shop", "0014_order_coupon")]

    operations = [
        # Only the choices change: Django runs no SQL.
        migrations.AlterField(
            model_name="customer",
            name="tier",
            field=models.CharField(choices=[("basic", "Basic"), ("gold", "Gold")], max_length=10),
        ),
        migrations.RunPython(default_tier, migrations.RunPython.noop),
    ]
