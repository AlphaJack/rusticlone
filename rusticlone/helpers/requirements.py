"""
Utility functions for version checking
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of requirements.py                                   │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── FUNCTIONS
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# rusticlone
from rusticlone.helpers.action import Action
from rusticlone.helpers.rclone import Rclone
from rusticlone.helpers.rustic import Rustic

# ################################################################ FUNCTIONS


def check_rustic_version() -> bool:
    """
    Check that the installed Rustic version is supported
    """
    action = Action("Checking Rustic version")
    rustic = Rustic("", "--version")
    version = (
        rustic.stdout.splitlines()[0].replace("rustic", "").replace("v", "").strip()
    )
    try:
        major_version = int(version.split(".")[0])
        minor_version = int(version.split(".")[1])
    except (ValueError, TypeError):
        return action.abort("Could not parse Rustic version")
    if major_version != 0 or minor_version != 9:
        return action.abort(
            f"Rustic {major_version}.{minor_version} is installed, but 0.9 is required"
        )
    return action.stop()


def check_rclone_version() -> bool:
    """
    Check that the installed Rclone version is supported
    """
    action = Action("Checking Rclone version")
    rclone = Rclone(default_flags=None)
    version = (
        rclone.stdout.splitlines()[0].replace("rclone", "").replace("v", "").strip()
    )
    try:
        major_version = int(version.split(".")[0])
        minor_version = int(version.split(".")[1])
    except (ValueError, TypeError):
        return action.abort("Could not parse Rclone version")
    if major_version <= 1 and minor_version < 67:
        return action.abort(
            f"Rclone {major_version}.{minor_version} is installed, but at least 1.67 is required"
        )
    return action.stop()
