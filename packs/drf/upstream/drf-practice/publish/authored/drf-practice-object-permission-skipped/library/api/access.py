def has_perms_owner_aware(user, perm, obj):
    return obj.owner_id == user.pk or user.has_perm(perm, obj)
