from django.db import migrations, models


def create_walk_in_order(apps, schema_editor):
    Customer = apps.get_model("shop", "Customer")
    Order = apps.get_model("shop", "Order")
    walk_in = Customer.objects.get(email="walk-in@example.com")
    Order.objects.get_or_create(customer=walk_in, number="WALK-IN")


class Migration(migrations.Migration):
    dependencies = [("shop", "0005_customer_email_index")]

    # The new order row has a foreign key, checked at commit; the ALTER TABLE comes first.
    operations = [
        migrations.RunPython(create_walk_in_order, migrations.RunPython.noop),  # caught
        migrations.SeparateDatabaseAndState(
            database_operations=[
                migrations.AlterField(
                    model_name="order",
                    name="number",
                    field=models.CharField(max_length=30, unique=True),
                ),
            ],
        ),
    ]
