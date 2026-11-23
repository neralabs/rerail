from celery import shared_task

from plates.models import Plate


@shared_task
def lock_plates(pk):
    # rebuild.py calls it inside transaction.atomic(). The run names this module backend.plates.gate while the import
    # says plates.gate, so the call graph has no edge for that call; a call spelled `lock_plates(...)` with no known
    # target still counts as a caller.
    list(Plate.objects.filter(pk=pk).select_for_update().values_list("pk"))
