from django.db import models


class Route(models.Model):
    """A route on the wall, while it's up."""

    name = models.CharField(max_length=80)
    grade = models.CharField(max_length=4)
    colour = models.CharField(max_length=20)
    setter = models.CharField(max_length=80)
    set_on = models.DateField()
    taken_down_on = models.DateField(null=True, blank=True)

    def __str__(self):
        return f"{self.name} ({self.grade})"
