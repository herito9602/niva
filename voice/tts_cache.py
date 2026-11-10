"""A small on-disk cache of synthesized replies (latency: a cache hit skips the network).

Most of Niva's spoken replies are template phrases ("Volumen al 30%.", "Abrí
Calculadora."), so the same text comes back again and again. After a COMPLETE
edge-tts synthesis of a short text, the raw MP3 is stored; the next time that
exact text is spoken it is decoded from disk and goes through the robot effect and
the output device as usual, with no network round trip.

What is cached: the raw MP3 from the service, BEFORE the robot effect, so the
effect switch keeps working and a cached clip is exactly what the network would
have delivered. The key is a hash of (engine, voice, prosody, output format, exact
text, ``CACHE_FORMAT_VERSION``); change any of them and the old entries are simply
never found.

Privacy: the cache holds Niva's own replies, never the user's speech. The files are
named by a hash, so the folder does not list the texts, but the audio of a reply
can be played back from it; replies can mention what was asked ("Abrí
Calculadora."). Only short texts are cached (search summaries and other long
replies are not), the folder is bounded, and ``--no-tts-cache`` or deleting the
folder turns it off.

Robustness: writes are atomic (temp file, then rename), every file carries its
length and CRC so a truncated or damaged file is deleted and resynthesized, and
the oldest files (by modification time, refreshed on every hit) are evicted when
the folder grows past its limit.
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import struct
import threading
import time
import zlib
from pathlib import Path

log = logging.getLogger("niva.voice.tts_cache")

# Bump when anything about the stored audio changes (voice settings, output format, file layout).
CACHE_FORMAT_VERSION = 1
EDGE_PROSODY = "rate=+0%;pitch=+0Hz;volume=+0%"  # what Communicate sends by default (edge_ws.py does too)
EDGE_OUTPUT_FORMAT = "audio-24khz-48kbitrate-mono-mp3"

_MAGIC = b"NVTC"
_HEADER = struct.Struct(">4sIQI")  # magic, format version, payload length, CRC32 of the payload
_SUFFIX = ".mp3c"
_MIN_AUDIO_BYTES = 256  # anything smaller is not a real clip


def default_cache_dir() -> Path | None:
    """``%LOCALAPPDATA%\\Niva\\tts-cache``; ``NIVA_TTS_CACHE_DIR`` overrides it. None when neither exists."""
    override = os.environ.get("NIVA_TTS_CACHE_DIR")
    if override:
        return Path(override)
    base = os.environ.get("LOCALAPPDATA")
    return Path(base) / "Niva" / "tts-cache" if base else None


class TtsCache:
    def __init__(self, directory: Path | str, max_bytes: int = 50 * 1024 * 1024, max_text_chars: int = 120):
        self._dir = Path(directory)
        self._max_bytes = max_bytes
        self._max_chars = max_text_chars
        self._lock = threading.Lock()

    @property
    def directory(self) -> Path:
        return self._dir

    def cacheable(self, text: str) -> bool:
        return 0 < len(text) <= self._max_chars

    @staticmethod
    def key(engine: str, voice: str, text: str) -> str:
        payload = json.dumps(
            [CACHE_FORMAT_VERSION, engine, voice, EDGE_PROSODY, EDGE_OUTPUT_FORMAT, text], ensure_ascii=False
        )
        return hashlib.sha256(payload.encode("utf-8")).hexdigest()

    def _path(self, key: str) -> Path:
        return self._dir / f"{key}{_SUFFIX}"

    def get(self, key: str) -> bytes | None:
        """The stored MP3, or None (absent, unreadable, truncated or damaged: the last three delete the file)."""
        path = self._path(key)
        try:
            raw = path.read_bytes()
        except FileNotFoundError:
            return None
        except OSError as exc:
            log.warning("tts cache: cannot read %s: %s", path.name, exc)
            return None
        audio = self._unpack(raw)
        if audio is None:
            log.warning("tts cache: dropping damaged entry %s", path.name)
            self.delete(key)
            return None
        try:
            os.utime(path)  # a hit keeps the entry young for the eviction order
        except OSError:
            pass
        return audio

    def put(self, key: str, audio: bytes) -> None:
        """Store a COMPLETE clip. Never raises: a cache that cannot write is just a cache miss."""
        if len(audio) < _MIN_AUDIO_BYTES:
            return
        path = self._path(key)
        temp = path.with_name(f"{path.name}.{os.getpid()}.{threading.get_ident()}.tmp")
        try:
            self._dir.mkdir(parents=True, exist_ok=True)
            temp.write_bytes(self._pack(audio))
            os.replace(temp, path)  # atomic: a reader sees the whole file or none
        except OSError as exc:
            log.warning("tts cache: cannot store a clip: %s", exc)
            try:
                temp.unlink(missing_ok=True)
            except OSError:
                pass
            return
        self._evict()

    def delete(self, key: str) -> None:
        try:
            self._path(key).unlink(missing_ok=True)
        except OSError as exc:
            log.warning("tts cache: cannot delete an entry: %s", exc)

    # -- file layout

    @staticmethod
    def _pack(audio: bytes) -> bytes:
        return _HEADER.pack(_MAGIC, CACHE_FORMAT_VERSION, len(audio), zlib.crc32(audio)) + audio

    @staticmethod
    def _unpack(raw: bytes) -> bytes | None:
        if len(raw) < _HEADER.size:
            return None
        magic, version, length, crc = _HEADER.unpack_from(raw)
        audio = raw[_HEADER.size :]
        if magic != _MAGIC or version != CACHE_FORMAT_VERSION or length != len(audio) or zlib.crc32(audio) != crc:
            return None
        return audio

    # -- size bound

    def _evict(self) -> None:
        """Delete the least recently used files until the folder is back under ~90% of its limit."""
        with self._lock:
            try:
                entries = []
                for item in self._dir.iterdir():
                    if item.name.endswith(".tmp"):
                        # Another writer may be mid-flight; one older than an hour is a crash leftover.
                        if time.time() - item.stat().st_mtime > 3600:
                            item.unlink(missing_ok=True)
                        continue
                    if item.suffix != _SUFFIX:
                        continue
                    stat = item.stat()
                    entries.append((stat.st_mtime, stat.st_size, item))
            except OSError as exc:
                log.warning("tts cache: cannot list the folder: %s", exc)
                return
            total = sum(size for _, size, _ in entries)
            if total <= self._max_bytes:
                return
            target = int(self._max_bytes * 0.9)
            for _, size, item in sorted(entries, key=lambda e: e[0]):
                if total <= target:
                    break
                try:
                    item.unlink()
                    total -= size
                except OSError:
                    pass
