from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("shop", "0006_widen")]

    operations = [
        migrations.CreateModel(
            name="SiteConfig",
            fields=[
                ("id", models.AutoField(auto_created=True, primary_key=True, serialize=False)),
                ("page_size", models.IntegerField(default=25)),
            ],
        ),
    ]
