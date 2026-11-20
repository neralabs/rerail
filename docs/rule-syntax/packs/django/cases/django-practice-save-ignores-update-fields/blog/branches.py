from django.db import models


class EarlyReturn(models.Model):
    code = models.CharField(max_length=10)

    # Updates return before the assignment: `code` is set only for a new row.
    def save(self, *args, **kwargs):
        if self.pk:
            return super().save(*args, **kwargs)
        self.code = "new"
        super().save(*args, **kwargs)


class ElseBranch(models.Model):
    created = models.DateTimeField(null=True)

    # The else branch of `if self.pk` is the new-row branch.
    def save(self, *args, **kwargs):
        if self.pk is not None:
            pass
        else:
            self.created = None
        super().save(*args, **kwargs)


class ElifBranch(models.Model):
    code = models.CharField(max_length=10)

    def save(self, *args, **kwargs):
        if kwargs.get("raw"):
            pass
        elif self._state.adding:
            self.code = "new"
        super().save(*args, **kwargs)


class ExistingBranch(models.Model):
    code = models.CharField(max_length=10)

    # The body of `if self.pk` runs on updates: reported.
    def save(self, *args, **kwargs):  # caught
        if self.pk:
            self.code = "changed"
        super().save(*args, **kwargs)


class ReturnWithElse(models.Model):
    code = models.CharField(max_length=10)

    # The if returns for updates, so the code after it runs only for a new row, else branch or not.
    def save(self, *args, **kwargs):
        if self.pk:
            return super().save(*args, **kwargs)
        else:
            pass
        self.code = "x"
        super().save(*args, **kwargs)


class NoReturn(models.Model):
    code = models.CharField(max_length=10)

    # The if does not return: the code after it runs on updates too.
    def save(self, *args, **kwargs):  # caught
        if self.pk:
            super().save(*args, **kwargs)
        self.code = "x"
        super().save(*args, **kwargs)
