from django.db import migrations, models


class Migration(migrations.Migration):
    initial = True

    dependencies = []

    operations = [
        migrations.CreateModel(
            name="Invoice",
            fields=[("id", models.AutoField(primary_key=True, serialize=False))],
        ),
    ]
