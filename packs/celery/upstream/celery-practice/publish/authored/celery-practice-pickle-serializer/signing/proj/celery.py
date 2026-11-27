# A run that signs its messages with app.setup_security(): pickle under message signing is left alone.
from celery import Celery

app = Celery("proj")
app.conf.update(
    security_key="/etc/ssl/private/worker.key",
    security_certificate="/etc/ssl/certs/worker.pem",
    security_cert_store="/etc/ssl/certs/*.pem",
    task_serializer="pickle",
    accept_content=["pickle"],
)
app.setup_security(allowed_serializers=["pickle"], serializer="pickle")
