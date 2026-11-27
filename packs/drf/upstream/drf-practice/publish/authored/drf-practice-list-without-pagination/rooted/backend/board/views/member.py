from rest_framework import serializers, viewsets

from board.models import Member


class MemberSerializer(serializers.ModelSerializer):
    class Meta:
        model = Member
        fields = ["id", "name"]


# Routed only to retrieve and partial_update: no list endpoint exists.
class MemberEndpoint(viewsets.ModelViewSet):
    queryset = Member.objects.all()
    serializer_class = MemberSerializer


# Routed to list through the re-exporting package.
class MemberAdminViewSet(viewsets.ModelViewSet):  # caught
    queryset = Member.objects.all()
    serializer_class = MemberSerializer
