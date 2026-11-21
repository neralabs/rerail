from django.db import models


class Post(models.Model):
    title = models.CharField(max_length=100)
    views = models.IntegerField(default=0)

    def bump(self):
        self.views += 1
        self.save()


class Visit(models.Model):
    post = models.ForeignKey(Post, on_delete=models.CASCADE)
