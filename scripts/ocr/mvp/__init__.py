"""Opt-in local OCR MVP. No cloud, implicit provider, or reference inputs."""
from .local import run_task, readiness, cancel_task
from .store import load_snapshot, review_save
__all__ = ['run_task', 'readiness', 'load_snapshot', 'review_save', 'cancel_task']
