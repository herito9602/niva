"""The on-disk cache of synthesized replies. Temp folders only; no network, no decoder."""

import os
import threading
from pathlib import Path

import pytest

import tts
from config import Tunables
from tts import EdgeTtsEngine, PcmChunk, Speaker
from tts_cache import CACHE_FORMAT_VERSION, TtsCache, default_cache_dir

AUDIO = bytes(range(256)) * 4  # 1 KiB stand-in for an MP3 clip
VOICE = "es-US-PalomaNeural"

# ------------------------------------------------------------------ the store


def test_a_stored_clip_comes_back_byte_for_byte(tmp_path):
    cache = TtsCache(tmp_path)
    key = cache.key("edge-tts", VOICE, "Volumen al 30%.")
    assert cache.get(key) is None
    cache.put(key, AUDIO)
    assert cache.get(key) == AUDIO
    assert [p.name for p in tmp_path.iterdir()] == [f"{key}.mp3c"], "no temp files are left behind"


def test_the_key_depends_on_engine_voice_and_the_exact_text():
    base = TtsCache.key("edge-tts", VOICE, "Hola.")
    assert base == TtsCache.key("edge-tts", VOICE, "Hola.")
    assert len({base, TtsCache.key("piper", VOICE, "Hola."), TtsCache.key("edge-tts", "en-US-AriaNeural", "Hola."),
                TtsCache.key("edge-tts", VOICE, "Hola"), TtsCache.key("edge-tts", VOICE, "hola.")}) == 5


def test_the_key_includes_a_cache_format_version(monkeypatch):
    import tts_cache

    old = TtsCache.key("edge-tts", VOICE, "Hola.")
    monkeypatch.setattr(tts_cache, "CACHE_FORMAT_VERSION", CACHE_FORMAT_VERSION + 1)
    assert TtsCache.key("edge-tts", VOICE, "Hola.") != old


def test_only_short_texts_are_cacheable(tmp_path):
    cache = TtsCache(tmp_path, max_text_chars=20)
    assert cache.cacheable("Hecho.") and cache.cacheable("x" * 20)
    assert not cache.cacheable("x" * 21) and not cache.cacheable("")


@pytest.mark.parametrize(
    "damage",
    [
        lambda raw: raw[: len(raw) // 2],  # truncated
        lambda raw: raw[:10],  # shorter than the header
        lambda raw: raw[:-1] + bytes([raw[-1] ^ 0xFF]),  # a flipped bit in the audio
        lambda raw: b"XXXX" + raw[4:],  # wrong magic
        lambda raw: b"",
    ],
)
def test_a_damaged_file_is_deleted_and_reads_as_a_miss(tmp_path, damage):
    cache = TtsCache(tmp_path)
    key = cache.key("edge-tts", VOICE, "Hola.")
    cache.put(key, AUDIO)
    path = tmp_path / f"{key}.mp3c"
    path.write_bytes(damage(path.read_bytes()))
    assert cache.get(key) is None
    assert not path.exists()


def test_a_clip_too_small_to_be_real_audio_is_not_stored(tmp_path):
    cache = TtsCache(tmp_path)
    cache.put("k", b"tiny")
    assert list(tmp_path.iterdir()) == []


def test_the_oldest_clips_are_evicted_when_the_folder_is_over_its_limit(tmp_path):
    cache = TtsCache(tmp_path, max_bytes=3 * 1100)
    keys = [cache.key("edge-tts", VOICE, f"Texto {i}.") for i in range(4)]
    for age, key in enumerate(keys[:3]):
        cache.put(key, AUDIO)
        os.utime(tmp_path / f"{key}.mp3c", (1_000 + age, 1_000 + age))  # keys[0] is the oldest
    assert cache.get(keys[0]) == AUDIO  # a hit makes it the youngest
    cache.put(keys[3], AUDIO)  # over the limit: the least recently used goes
    survivors = {p.name for p in tmp_path.iterdir()}
    assert f"{keys[1]}.mp3c" not in survivors
    assert {f"{keys[0]}.mp3c", f"{keys[3]}.mp3c"} <= survivors


def test_a_cache_that_cannot_write_is_a_miss_not_an_error(tmp_path):
    blocker = tmp_path / "not-a-folder"
    blocker.write_bytes(b"x")
    cache = TtsCache(blocker / "tts-cache")
    cache.put("k", AUDIO)  # must not raise
    assert cache.get("k") is None


def test_the_default_folder_follows_the_app_data_convention(monkeypatch, tmp_path):
    monkeypatch.delenv("NIVA_TTS_CACHE_DIR", raising=False)
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path))
    assert default_cache_dir() == tmp_path / "Niva" / "tts-cache"
    monkeypatch.setenv("NIVA_TTS_CACHE_DIR", str(tmp_path / "elsewhere"))
    assert default_cache_dir() == tmp_path / "elsewhere"
    monkeypatch.delenv("NIVA_TTS_CACHE_DIR")
    monkeypatch.delenv("LOCALAPPDATA")
    assert default_cache_dir() is None


# ------------------------------------------------------------------ in the engine


@pytest.fixture
def fake_decoder(monkeypatch):
    """Decoding is not under test: the 'PCM' is the MP3 bytes, in 256-byte blocks, and garbage fails."""

    def decode(chunks, sample_rate=24_000):
        data = b"".join(chunks)
        if data.startswith(b"GARBAGE"):
            raise ValueError("not an MP3")
        for i in range(0, len(data), 256):
            yield PcmChunk(data[i : i + 256], sample_rate)

    monkeypatch.setattr(tts, "decode_mp3_stream", decode)


class CountingNetwork:
    def __init__(self, payload=AUDIO, fail_after=None, piece=256):
        self.payload, self.fail_after, self.piece = payload, fail_after, piece
        self.calls = []

    async def __call__(self, text, voice):
        self.calls.append((text, voice))
        for i in range(0, len(self.payload), self.piece):
            if self.fail_after is not None and i >= self.fail_after:
                raise ConnectionError("lost the connection")
            yield self.payload[i : i + self.piece]


def pcm(engine, text="Hola.", lang="es", cancel=None):
    return b"".join(c.data for c in engine.chunks(text, lang, cancel or threading.Event()))


def test_a_complete_synthesis_is_stored_and_the_repeat_needs_no_network(tmp_path, fake_decoder):
    network = CountingNetwork()
    engine = EdgeTtsEngine(Tunables(), network, cache=TtsCache(tmp_path))
    first = pcm(engine)
    engine.wait_for_cache()
    second = pcm(engine)
    assert first == second == AUDIO
    assert network.calls == [("Hola.", VOICE)], "the second reply came from the cache"


def test_the_cache_stores_the_raw_mp3_so_the_robot_effect_still_applies_to_hits(tmp_path, fake_decoder):
    cache = TtsCache(tmp_path)
    engine = EdgeTtsEngine(Tunables(), CountingNetwork(), cache=cache)
    pcm(engine)
    engine.wait_for_cache()
    assert cache.get(cache.key("edge-tts", VOICE, "Hola.")) == AUDIO  # untouched by any effect


def test_each_language_has_its_own_entry(tmp_path, fake_decoder):
    network = CountingNetwork()
    engine = EdgeTtsEngine(Tunables(), network, cache=TtsCache(tmp_path))
    pcm(engine, "Ok.", "es")
    pcm(engine, "Ok.", "en")
    assert [voice for _, voice in network.calls] == [tts.EDGE_VOICES["es"], tts.EDGE_VOICES["en"]]


def test_long_texts_are_never_cached(tmp_path, fake_decoder):
    network = CountingNetwork()
    cache = TtsCache(tmp_path, max_text_chars=10)
    engine = EdgeTtsEngine(Tunables(), network, cache=cache)
    pcm(engine, "Este texto es largo.")
    pcm(engine, "Este texto es largo.")
    assert len(network.calls) == 2 and not list(tmp_path.iterdir())


def test_a_synthesis_that_dies_midway_is_not_cached(tmp_path, fake_decoder):
    engine = EdgeTtsEngine(Tunables(), CountingNetwork(fail_after=512), cache=TtsCache(tmp_path))
    with pytest.raises(ConnectionError):
        pcm(engine)
    engine.wait_for_cache()
    assert not list(tmp_path.iterdir())


def test_a_stopped_utterance_is_not_cached(tmp_path, fake_decoder):
    cancel = threading.Event()
    engine = EdgeTtsEngine(Tunables(), CountingNetwork(), cache=TtsCache(tmp_path))
    stream = engine.chunks("Hola.", "es", cancel)
    next(stream)
    cancel.set()
    stream.close()  # the speaker stopped reading
    engine.wait_for_cache()
    assert not list(tmp_path.iterdir())


def test_a_clip_that_does_not_decode_is_dropped_and_synthesized_again(tmp_path, fake_decoder):
    cache = TtsCache(tmp_path)
    key = cache.key("edge-tts", VOICE, "Hola.")
    cache.put(key, b"GARBAGE" + bytes(400))
    network = CountingNetwork()
    engine = EdgeTtsEngine(Tunables(), network, cache=cache)
    assert pcm(engine) == AUDIO
    engine.wait_for_cache()
    assert len(network.calls) == 1 and cache.get(key) == AUDIO, "replaced by the good clip"


def test_a_damaged_file_on_disk_is_synthesized_again(tmp_path, fake_decoder):
    cache = TtsCache(tmp_path)
    key = cache.key("edge-tts", VOICE, "Hola.")
    cache.put(key, AUDIO)
    path = tmp_path / f"{key}.mp3c"
    path.write_bytes(path.read_bytes()[:100])
    network = CountingNetwork()
    assert pcm(EdgeTtsEngine(Tunables(), network, cache=cache)) == AUDIO
    assert len(network.calls) == 1


def test_without_a_cache_every_reply_is_synthesized(tmp_path, fake_decoder):
    network = CountingNetwork()
    engine = EdgeTtsEngine(Tunables(), network)
    pcm(engine)
    pcm(engine)
    assert len(network.calls) == 2


def test_a_cache_hit_plays_through_the_speaker_with_no_network(tmp_path, fake_decoder):
    cache = TtsCache(tmp_path)
    network = CountingNetwork(payload=bytes([200, 15] * 600))  # loud samples
    engine = EdgeTtsEngine(Tunables(), network, cache=cache)
    log, ended = [], []

    class Out:
        def __init__(self, rate):
            self.sample_rate = rate

        def write(self, data):
            log.append(len(data))

        def abort(self):
            pass

        def close(self):
            pass

    speaker = Speaker([engine], Out, lambda m: log.append(m["type"]), ended.append, Tunables(robot_voice=False))
    for speak_id in (1, 2):
        speaker.speak(speak_id, "Hola.", "es")
        speaker.join(3)
        engine.wait_for_cache()
    assert len(network.calls) == 1 and ended == [1, 2]
    assert log.count("speak_started") == 2 and log.count("speak_done") == 2


SAMPLE_MP3 = Path(__file__).resolve().parents[2] / "spikes" / "voice" / "s1_audio" / "en00.mp3"


@pytest.mark.skipif(not SAMPLE_MP3.exists(), reason="local TTS sample (spikes/voice/s1_audio) not present")
def test_a_cached_real_mp3_decodes_to_the_same_audio_as_the_streamed_one(tmp_path):
    data = SAMPLE_MP3.read_bytes()
    network = CountingNetwork(payload=data, piece=4096)  # network-sized pieces
    engine = EdgeTtsEngine(Tunables(), network, cache=TtsCache(tmp_path))
    streamed = pcm(engine, "Hello.", "en")
    engine.wait_for_cache()
    cached = pcm(engine, "Hello.", "en")
    assert streamed == cached and len(streamed) > 24_000 and len(network.calls) == 1


def test_a_slow_cache_write_does_not_hold_up_the_end_of_the_reply(tmp_path, fake_decoder):
    """The write runs after the last chunk was handed over, on its own thread: the speaker flushes
    the effect tail and drains the device without waiting for the disk."""
    release = threading.Event()
    started = threading.Event()

    class SlowCache(TtsCache):
        def put(self, key, audio):
            started.set()
            release.wait(5)
            super().put(key, audio)

    cache = SlowCache(tmp_path)
    engine = EdgeTtsEngine(Tunables(), CountingNetwork(), cache=cache)
    stream = engine.chunks("Hola.", "es", threading.Event())
    assert b"".join(c.data for c in stream) == AUDIO  # the generator finished while put is still blocked
    assert started.wait(2) and cache.get(cache.key("edge-tts", VOICE, "Hola.")) is None
    release.set()
    engine.wait_for_cache()
    assert cache.get(cache.key("edge-tts", VOICE, "Hola.")) == AUDIO


def test_the_speaker_finishes_and_says_done_while_the_cache_write_is_still_blocked(tmp_path, fake_decoder):
    release = threading.Event()

    class SlowCache(TtsCache):
        def put(self, key, audio):
            release.wait(5)
            super().put(key, audio)

    engine = EdgeTtsEngine(Tunables(), CountingNetwork(payload=bytes([200, 15] * 600)), cache=SlowCache(tmp_path))
    seen, ended = [], []

    class Out:
        def __init__(self, rate):
            self.sample_rate = rate

        def write(self, data):
            seen.append("write")

        def abort(self):
            pass

        def close(self):
            seen.append("close")

    speaker = Speaker([engine], Out, lambda m: seen.append(m["type"]), ended.append, Tunables(robot_voice=False))
    speaker.speak(1, "Hola.", "es")
    speaker.join(3)
    assert not speaker._thread.is_alive() and ended == [1]  # the write is still blocked, the reply is over
    assert seen[-2:] == ["close", "speak_done"]
    release.set()
    engine.wait_for_cache()
