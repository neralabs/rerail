from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0008_siteconfig_bigint")]

    operations = [
        migrations.AddField(model_name="order", name="customer_ref", field=models.IntegerField(null=True)),
        migrations.AddField(model_name="order", name="draft_ref", field=models.IntegerField(null=True)),
        # customer.id is a BigAutoField since 0003: the integer column becomes a bigint.
        migrations.AlterField(
            model_name="order",
            name="customer_ref",
            field=models.ForeignKey(null=True, on_delete=models.SET_NULL, to="shop.customer"),  # caught
        ),
        # draft.id is still an AutoField: integer to integer.
        migrations.AlterField(
            model_name="order",
            name="draft_ref",
            field=models.ForeignKey(null=True, on_delete=models.SET_NULL, to="shop.draft"),
        ),
        # The same target: the column keeps its type.
        migrations.AlterField(
            model_name="order",
            name="customer",
            field=models.ForeignKey(on_delete=models.PROTECT, related_name="orders", to="shop.customer"),
        ),
    ]
