from fastapi import Depends, Request


def get_current_user(request: Request):
    return request.state.user


class PermissionsDependency:
    def __init__(self, permissions):
        self.permissions = permissions

    def __call__(self, request: Request, user=Depends(get_current_user)):
        return user
