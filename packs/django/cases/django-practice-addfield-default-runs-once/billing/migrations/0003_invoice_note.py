from django.db import migrations, models


class Migration(migrations.Migration):
    dependencies = [("billing", "0002_invoice_secret")]

    operations = [
        migrations.AddField(
            model_name="invoice",
            name="note",
            field=models.TextField(default=""),
        ),
    ]
