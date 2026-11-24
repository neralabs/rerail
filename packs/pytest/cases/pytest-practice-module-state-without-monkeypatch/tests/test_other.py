from tests.test_state import Recorder


def test_reads_recorder():
    assert Recorder.events == []
