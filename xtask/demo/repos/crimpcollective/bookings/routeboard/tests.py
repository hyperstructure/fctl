from datetime import date

from django.test import TestCase

from .models import Route


class BoardTests(TestCase):
    def test_the_board_shows_routes_that_are_up(self):
        Route.objects.create(name="Pinch me", grade="6a", colour="yellow", setter="Elsa", set_on=date(2026, 3, 1))
        Route.objects.create(
            name="Gone", grade="5c", colour="blue", setter="Elsa", set_on=date(2026, 1, 1), taken_down_on=date(2026, 2, 1)
        )
        page = self.client.get("/routes/board")
        self.assertContains(page, "Pinch me")
        self.assertNotContains(page, "Gone")

    def test_a_route_names_its_grade(self):
        r = Route(name="Slab city", grade="5b", colour="green", setter="Ola", set_on=date(2026, 3, 1))
        self.assertEqual(str(r), "Slab city (5b)")
