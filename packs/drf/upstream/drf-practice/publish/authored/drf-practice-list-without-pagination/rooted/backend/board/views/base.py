from rest_framework import generics

from board.models import Card


# A base view of the run: reported through its subclasses, which it reaches under the project's import root.
class CardListBase(generics.ListAPIView):
    queryset = Card.objects.all()
