from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0007_siteconfig")]

    operations = [
        # shop/config.py reads SiteConfig as one row: nothing to lock for long.
        migrations.AlterField(
            model_name="siteconfig",
            name="page_size",
            field=models.BigIntegerField(default=25),
        ),
        # Customer is an ordinary table: the same change rewrites it.
        migrations.AlterField(
            model_name="customer",
            name="name",
            field=models.CharField(max_length=50),  # caught
        ),
    ]
