from django.db import migrations, models


def set_currency(apps, schema_editor):
    SiteSettings = apps.get_model("shop", "SiteSettings")
    for site in SiteSettings.objects.all():
        site.currency = "EUR"
        site.save()


class Migration(migrations.Migration):
    dependencies = [("shop", "0015_customer_tier")]

    # shop/views.py reads SiteSettings as one row: its table has one row.
    operations = [
        migrations.AddField(model_name="sitesettings", name="currency", field=models.CharField(max_length=3, null=True)),
        migrations.RunPython(set_currency, migrations.RunPython.noop),
    ]
