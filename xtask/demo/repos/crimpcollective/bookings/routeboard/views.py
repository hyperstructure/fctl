from django.shortcuts import render

from .models import Route


def board(request):
    """The screen by the wall: every route that's up, newest first."""
    routes = Route.objects.filter(taken_down_on__isnull=True).order_by("-set_on")
    return render(request, "routeboard/board.html", {"routes": routes})
