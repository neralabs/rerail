from django.db import models


class LockModel:
    def refresh_for_update(self):
        # A model mixin's method nobody in the project calls: an API whose callers hold the transaction.
        return type(self).objects.filter(pk=self.pk).select_for_update().get()


class Hold(LockModel, models.Model):
    seats = models.IntegerField(default=0)

    def lock_again(self):
        # A model method nobody in the project calls.
        return type(self).objects.filter(pk=self.pk).select_for_update().get()
