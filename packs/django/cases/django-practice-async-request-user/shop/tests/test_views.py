async def test_user(request):
    assert request.user.is_authenticated
