"""Python wrapper for the linux-x11-harness native automation driver."""

from pathlib import Path


def get_binary_path() -> Path:
    """Return the path to the bundled linux-x11-harness executable."""
    return Path(__file__).parent / "bin" / "linux-x11-harness"


def get_skill_path() -> Path:
    """Return the path to the bundled Agent Skill directory."""
    return Path(__file__).parent / "skill"


def get_lifecycle_path() -> Path:
    """Return the path to the bundled per-host lifecycle shell directory."""
    return Path(__file__).parent / "lifecycle"
