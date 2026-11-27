from rest_framework import serializers, viewsets

from notes.models import Note


class NoteSerializer(serializers.ModelSerializer):
    class Meta:
        model = Note
        fields = ["id", "body"]


class NoteViewSet(viewsets.ModelViewSet):  # caught
    queryset = Note.objects.all()
    serializer_class = NoteSerializer
