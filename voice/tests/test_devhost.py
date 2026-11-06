import queue

from devhost import wrap_emit
from protocol import Listen, Speak


def test_dev_host_listens_after_wake_and_speaks_the_transcript_back():
    sent, inbox = [], queue.Queue()
    emit = wrap_emit(sent.append, inbox)
    emit({"type": "wake", "score": 0.8})
    emit({"type": "transcript", "id": 1, "text": "open spotify", "lang": "en", "lang_prob": 1.0, "end_ms": 0})
    emit({"type": "transcript", "id": 2, "text": "", "lang": "en", "lang_prob": 1.0, "end_ms": 0})
    assert [m["type"] for m in sent] == ["wake", "transcript", "transcript"]
    assert inbox.get_nowait() == ("cmd", Listen(1, "command"))
    assert inbox.get_nowait() == ("cmd", Speak(2, "I heard: open spotify", "en"))
    assert inbox.empty()
