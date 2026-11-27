from celery import signature

from proj import contract
from proj.celery import app
from proj.contract import GONE_TASK
from proj.tasks import ExportTask

GONE = "proj.tasks.gone"
SHIP = "proj.tasks.ship"


def dispatch(order_id, kind):
    app.send_task("proj.tasks.ship", args=[order_id])
    app.send_task("proj.tasks.gone", args=[order_id])  # caught
    app.send_task(name="proj.tasks.helper", args=[order_id])  # caught
    app.send_task(GONE, args=[order_id])  # caught
    app.send_task(SHIP, args=[order_id])
    app.send_task(f"proj.tasks.{kind}", args=[order_id])
    app.send_task(kind, args=[order_id])
    app.send_task("billing.tasks.charge", args=[order_id])
    signature("proj.tasks.ship", args=(order_id,)).delay()
    signature("proj.maintenance.clean", args=(order_id,)).delay()  # caught
    app.signature("reports.scheduler").delay()
    app.signature("reports.prune").delay()  # caught


def dispatch_by_constant(order_id):
    app.send_task(contract.GONE_TASK, args=[order_id])  # caught
    app.send_task(GONE_TASK, args=[order_id])  # caught
    app.send_task(ExportTask.name, args=[order_id])
    app.send_task(contract.OPERATION_TASK, args=[order_id])
    local_name = "proj.tasks.gone_local"
    app.send_task(local_name, args=[order_id])  # caught
