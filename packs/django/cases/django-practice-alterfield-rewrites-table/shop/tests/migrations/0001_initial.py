from django.db import migrations, models


class Migration(migrations.Migration):
    initial = True

    dependencies = []

    operations = [
        migrations.CreateModel(
            name="Widget",
            fields=[("id", models.AutoField(auto_created=True, primary_key=True, serialize=False))],
        ),
    ]
