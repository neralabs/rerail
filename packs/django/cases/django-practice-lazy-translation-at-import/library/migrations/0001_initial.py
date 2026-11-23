from django.db import migrations, models
from django.utils.translation import gettext as _


class Migration(migrations.Migration):
    initial = True
    dependencies = []
    # Migrations are frozen history: skipped.
    operations = [
        migrations.CreateModel(name="Book", fields=[("title", models.CharField(_("title"), max_length=80))]),
    ]
