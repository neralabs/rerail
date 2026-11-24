from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("tests", "0001_initial")]

    operations = [
        migrations.AlterField(
            model_name="widget",
            name="id",
            field=models.BigAutoField(auto_created=True, primary_key=True, serialize=False),
        ),
    ]
