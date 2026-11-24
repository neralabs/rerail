from rest_framework import permissions


class HasObjectPermission(permissions.BasePermission):
    def has_object_permission(self, request, view, obj):
        return obj.has_permission(request.user)


class IsOwner(permissions.BasePermission):
    def has_object_permission(self, request, view, obj):
        return obj.owner == request.user


class IsStrictOwner(IsOwner):
    pass


class IsStaff(permissions.BasePermission):
    def has_permission(self, request, view):
        return request.user.is_staff


# Its object check never reads the object: nothing about the row is checked.
class IsMember(permissions.BasePermission):
    def has_object_permission(self, request, view, obj):
        return request.user.groups.filter(name="member").exists()
