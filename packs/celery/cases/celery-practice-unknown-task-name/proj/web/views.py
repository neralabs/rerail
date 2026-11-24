from celery import signature

from proj.celery import app

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
