"""
Utility functions for text formatting
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of formatting.py                                     │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── FUNCTIONS
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# os
import platform

# ################################################################ FUNCTIONS


def clear_line(n: int = 1, parallel: bool = False) -> None:
    """
    Go up and clear line, replacing a "wait" with an "ok"
    Not being used on Windows and parallel processing
    """
    line_up = "\033[1A"
    line_clear = "\x1b[2K"
    if platform.system() != "Windows" and not parallel:
        for i in range(n):
            print(line_up, end=line_clear)


def print_stats(
    content_left: str,
    content_right: str,
    width_left: int = 30,
    width_right: int = 10,
    parallel: bool = False,
) -> None:
    """
    Print left aligned and right aligned text
    Since it would mess the output in windows powershell, we use a workaround
    """
    # width_left = 80 - len(content_left)
    # width_right = 80 - len(content_right)
    begin = ""
    end = "\n"
    if platform.system() == "Windows":
        end = "\r"
        if content_right != "[OK]":
            begin = "\n"
    # if not async (global)
    if not parallel:
        # if True:
        if content_right != "":
            print(
                f"{begin}{content_left:<{width_left}}{content_right:>{width_right}}",
                end=end,
            )
        else:
            print(f"{begin}{content_left}")


def convert_size(size: int) -> str:
    """
    Convert a size in bytes to a human-readable format (e.g., KB, MB, GB, TB).
    """
    sep = ""
    # sep = " "
    units = ("KB", "MB", "GB", "TB")
    size_list = [f"{int(size):,}{sep}B"] + [
        f"{int(size) / 1024 ** (i + 1):,.0f}{sep}{u}" for i, u in enumerate(units)
    ]
    if size == 0:
        return f"0{sep}B"
    else:
        return [size for size in size_list if not size.startswith("0")][-1]
