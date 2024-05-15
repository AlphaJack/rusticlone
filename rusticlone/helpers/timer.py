"""
Simple stopwatch to measure time elapsed
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of timer.py                                          │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── CLASSES
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# stopwatch
import time

# rusticlone
from rusticlone.helpers.formatting import print_stats

# ################################################################ CLASSES


class Timer:
    """
    Simple stopwatch to measure time elapsed
    """

    def __init__(self, parallel: bool = False) -> None:
        """
        Initializes the timer using the time.perf_counter() function.
        """
        self.start_time = time.perf_counter()
        self.parallel = parallel
        self.stop_time = self.start_time
        self.duration = "0s"

    def stop(self) -> None:
        """
        Stop the timer and print the result
        """
        self.stop_time = time.perf_counter()
        self.duration = str(round(self.stop_time - self.start_time)) + "s"
        print_stats("Duration:", self.duration, parallel=self.parallel)
