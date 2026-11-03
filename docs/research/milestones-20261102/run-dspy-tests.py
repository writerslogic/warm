"""Run local DSPy integration cases with the documented memory-only cache."""
import sys

import dspy
import pytest

dspy.configure_cache(enable_disk_cache=False, enable_memory_cache=True)
raise SystemExit(pytest.main(sys.argv[1:]))
