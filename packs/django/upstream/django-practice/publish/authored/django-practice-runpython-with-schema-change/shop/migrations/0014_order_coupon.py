from django.db import migrations, models


def assign_coupons(apps, schema_editor):
    Coupon = apps.get_model("shop", "Coupon")
    Order = apps.get_model("shop", "Order")
    Order.objects.filter(coupon=None).update(coupon=Coupon.objects.get(code="WELCOME"))


class Migration(migrations.Migration):
    dependencies = [("shop", "0013_no_pending_checks")]

    # Django adds the coupon foreign key with SET CONSTRAINTS ... IMMEDIATE, so the update queues no check for it;
    # the AddField's lock on shop_order still lasts through the backfill.
    operations = [
        migrations.AddField(
            model_name="order",
            name="coupon",
            field=models.ForeignKey(null=True, on_delete=models.PROTECT, to="shop.coupon"),
        ),
        migrations.RunPython(assign_coupons, migrations.RunPython.noop),  # caught
        migrations.AlterField(
            model_name="order",
            name="coupon",
            field=models.ForeignKey(on_delete=models.PROTECT, to="shop.coupon"),
        ),
    ]
