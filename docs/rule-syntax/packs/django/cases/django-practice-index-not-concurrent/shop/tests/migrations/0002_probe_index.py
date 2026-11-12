from django.db import migrations, models


class Migration(migrations.Migration):
    operations = [
        migrations.AddIndex(model_name="probe", index=models.Index(fields=["id"], name="probe_idx")),
    ]
