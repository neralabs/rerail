import uuid

from django.db import migrations, models


class Migration(migrations.Migration):
    # A squash runs only on a database that applied neither 0002 nor 0003; 0002 is reported itself.
    replaces = [("billing", "0002_invoice_secret"), ("billing", "0003_invoice_note")]

    dependencies = [("billing", "0001_initial")]

    operations = [
        migrations.AddField(
            model_name="invoice",
            name="secret",
            field=models.UUIDField(default=uuid.uuid4),
        ),
        migrations.AddField(
            model_name="invoice",
            name="note",
            field=models.TextField(default=""),
        ),
        migrations.AddField(
            model_name="invoice",
            name="number",
            field=models.CharField(default="0", max_length=12, unique=True),
        ),
    ]
