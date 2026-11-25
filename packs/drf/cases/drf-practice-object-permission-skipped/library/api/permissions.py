from rest_framework import permissions
from rest_framework.permissions import SAFE_METHODS as READ_METHODS

SAFE = ("GET", "HEAD", "OPTIONS")


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


# Open to every safe request before obj is read (DRF's documented IsOwnerOrReadOnly).
class IsOwnerOrReadOnly(permissions.BasePermission):
    def has_object_permission(self, request, view, obj):
        if request.method in permissions.SAFE_METHODS:
            return True
        return obj.owner == request.user


# The same through an alias of SAFE_METHODS, as one `or`, after a docstring.
class ReadOrOwner(permissions.BasePermission):
    def has_object_permission(self, req, view, obj):
        """Anyone reads; the owner writes."""
        return req.method in READ_METHODS or obj.owner == req.user


# The same through a module constant.
class ConstReadOrOwner(permissions.BasePermission):
    def has_object_permission(self, request, view, obj):
        if request.method in SAFE:
            return True
        return obj.owner == request.user


# Inherits IsOwnerOrReadOnly's check.
class ShelfOwnerOrReadOnly(IsOwnerOrReadOnly):
    pass


# Named "read only", but its body checks the owner on every request.
class OwnerOrReadOnly(permissions.BasePermission):
    def has_object_permission(self, request, view, obj):
        return obj.owner == request.user


# Safe requests still depend on the row.
class PublicOrOwner(permissions.BasePermission):
    def has_object_permission(self, request, view, obj):
        if request.method in permissions.SAFE_METHODS:
            return obj.is_public
        return obj.owner == request.user


# Refuses some safe requests before the safe-method test.
class LiveOwnerOrReadOnly(permissions.BasePermission):
    def has_object_permission(self, request, view, obj):
        if obj.archived:
            return False
        if request.method in permissions.SAFE_METHODS:
            return True
        return obj.owner == request.user


# DjangoObjectPermissions with read permissions added: GET checks the row.
class ViewObjectPermissions(permissions.DjangoObjectPermissions):
    perms_map = {**permissions.DjangoObjectPermissions.perms_map, "GET": ["%(app_label)s.view_%(model_name)s"]}
