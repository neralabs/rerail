import pickle

from jinja2 import Template


def test_restore(rf):
    request = rf.post("/restore", data=b"...")
    assert pickle.loads(request.body) is not None
    assert Template(request.body.decode()).render() == ""
    assert eval(request.GET.get("x", "1")) == 1
