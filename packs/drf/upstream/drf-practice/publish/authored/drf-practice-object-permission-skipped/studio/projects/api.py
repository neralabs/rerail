# The source root is studio/: `projects.models` is studio/projects/models.py.
from rest_framework import generics
from rest_framework.exceptions import PermissionDenied
from rest_framework.response import Response

from projects.models import Project
from projects.serializers import ProjectSerializer


# The view's model comes from its serializer, imported from the same source root; the default permission applies.
class ProjectDetail(generics.RetrieveUpdateAPIView):
    serializer_class = ProjectSerializer

    def get(self, request, pk):
        project = generics.get_object_or_404(Project, pk=pk)  # caught
        return Response(ProjectSerializer(project).data)

    def put(self, request, pk):
        project = Project.objects.get(pk=pk)
        if not project.has_permission(request.user):
            return Response(status=403)
        return Response(ProjectSerializer(project).data)

    def patch(self, request, pk):
        project = Project.objects.get(pk=pk)
        if project.organization_id != request.user.active_organization_id:
            raise PermissionDenied()
        return Response(ProjectSerializer(project).data)


class ProjectByQueryset(generics.RetrieveAPIView):
    queryset = Project.objects.all()
    serializer_class = ProjectSerializer

    def get(self, request, pk):
        project = Project.objects.get(pk=pk)  # caught
        return Response(ProjectSerializer(project).data)
