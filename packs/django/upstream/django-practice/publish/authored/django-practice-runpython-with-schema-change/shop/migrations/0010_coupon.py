from django.db import migrations, models


def create_coupons(apps, schema_editor):
    Coupon = apps.get_model("shop", "Coupon")
    Coupon.objects.bulk_create([Coupon(code="WELCOME"), Coupon(code="SPRING")])


class Migration(migrations.Migration):
    dependencies = [("shop", "0009_read_and_other_model")]

    # A new, empty table: no other transaction uses it.
    operations = [
        migrations.CreateModel(
            name="Coupon",
            fields=[("id", models.AutoField(primary_key=True, serialize=False))],
        ),
        migrations.AddField(model_name="coupon", name="code", field=models.CharField(max_length=20)),
        migrations.RunPython(create_coupons, migrations.RunPython.noop),
    ]
