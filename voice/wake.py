"""Wake word detection: openWakeWord's stock ``hey_jarvis`` as a placeholder.

The custom "hey niva" model replaces it in slice 5 (design D6). The detector
sees the same int16 frames as the VAD; it buffers them internally into its own
80 ms windows, so a score only changes when a window completes.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Protocol

import numpy as np

WAKE_MODEL_NAME = "hey_jarvis"
# File names openWakeWord ships; the directory comes from configuration.
WAKE_MODEL_FILE = "hey_jarvis_v0.1.onnx"
MELSPEC_FILE = "melspectrogram.onnx"
EMBEDDING_FILE = "embedding_model.onnx"

WAKE_DIR_ENV = "NIVA_WAKE_MODEL_DIR"


class WakeDetector(Protocol):
    def score(self, frame: np.ndarray) -> float:
        """Score one int16 frame in [0, 1]."""
        ...

    def reset(self) -> None: ...


class WakeModelError(RuntimeError):
    pass


def find_wake_dir(configured: str | None) -> Path:
    """The directory holding the three openWakeWord ONNX files.

    Order: the explicit setting, ``NIVA_WAKE_MODEL_DIR``, then openWakeWord's own
    ``resources/models`` folder. Nothing is downloaded here.
    """
    candidates: list[Path] = []
    if configured:
        candidates.append(Path(configured))
    if os.environ.get(WAKE_DIR_ENV):
        candidates.append(Path(os.environ[WAKE_DIR_ENV]))
    try:
        import openwakeword

        candidates.append(Path(openwakeword.__file__).parent / "resources" / "models")
    except ImportError:  # pragma: no cover - the dependency is declared
        pass
    for folder in candidates:
        if all((folder / name).is_file() for name in (WAKE_MODEL_FILE, MELSPEC_FILE, EMBEDDING_FILE)):
            return folder
    tried = ", ".join(str(c) for c in candidates) or "none"
    raise WakeModelError(
        f"openWakeWord files ({WAKE_MODEL_FILE}, {MELSPEC_FILE}, {EMBEDDING_FILE}) not found; tried: {tried}. "
        f"Set {WAKE_DIR_ENV} or run openwakeword.utils.download_models once."
    )


class OpenWakeWordDetector:
    def __init__(self, model_dir: str | None = None):
        from openwakeword.model import Model

        folder = find_wake_dir(model_dir)
        self._model = Model(
            wakeword_models=[str(folder / WAKE_MODEL_FILE)],
            inference_framework="onnx",
            melspec_model_path=str(folder / MELSPEC_FILE),
            embedding_model_path=str(folder / EMBEDDING_FILE),
        )

    def score(self, frame: np.ndarray) -> float:
        return float(max(self._model.predict(frame).values(), default=0.0))

    def reset(self) -> None:
        self._model.reset()
