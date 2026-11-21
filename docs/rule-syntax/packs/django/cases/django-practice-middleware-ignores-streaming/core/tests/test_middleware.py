def test_size(client):
    def middleware(request):
        response = get_response(request)
        return response.content
