"""
Define which tasks are needed for archive, upload, download and extract operations
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of atomic.py                                         │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├──┐FUNCTIONS
# │  ├── BACKUP
# │  └── RESTORE
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# file locations
from pathlib import Path

# rusticlone
from rusticlone.helpers.action import Action
from rusticlone.helpers.timer import Timer
from rusticlone.processing.profile import Profile

# ################################################################ FUNCTIONS
# ################################ BACKUP


def profile_archive(
    name: str, log_file: Path, parallel: bool = False
) -> tuple[bool, str]:
    """
    Create a snapshot of a profile in a local rustic repo
    """
    action = Action(name, parallel, "[archive]")
    timer = Timer(parallel)
    profile = Profile(name, parallel)
    profile.read_rustic_config()
    profile.parse_rustic_config()
    profile.set_log_file(log_file)
    profile.check_source_exists()
    profile.check_local_repo_exists()
    profile.check_local_repo_health()
    profile.init()
    profile.backup()
    profile.forget()
    profile.source_stats()
    profile.repo_stats()
    timer.stop()
    # action.stop(" ", "")
    return profile.result, timer.duration


def profile_upload(
    name: str, log_file: Path, remote_prefix: str, parallel: bool = False
) -> tuple[bool, str]:
    """
    Sync the local rustic repo of a profile to a RClone remote
    """
    action = Action(name, parallel, "[upload]")
    timer = Timer(parallel)
    profile = Profile(name, parallel)
    profile.read_rustic_config()
    profile.parse_rustic_config()
    profile.parse_rclone_config()
    profile.set_log_file(log_file)
    profile.check_local_repo_exists()
    profile.upload(remote_prefix)
    timer.stop()
    # action.stop(" ", "")
    return profile.result, timer.duration


# ################################ RESTORE


def profile_download(
    name: str, log_file: Path, remote_prefix: str, parallel: bool = False
) -> tuple[bool, str]:
    """
    Retrieve the RClone remote of a profile to its local rustic repo location
    """
    action = Action(name, parallel, "[download]")
    timer = Timer(parallel)
    profile = Profile(name, parallel)
    profile.read_rustic_config()
    profile.parse_rustic_config()
    profile.parse_rclone_config()
    profile.set_log_file(log_file)
    profile.check_remote_repo_exists(remote_prefix)
    profile.check_local_repo_exists()
    profile.download(remote_prefix)
    timer.stop()
    # print_stats("", "")
    return profile.result, timer.duration


def profile_extract(
    name: str, log_file: Path, parallel: bool = False
) -> tuple[bool, str]:
    """
    Extract the latest snapshot of the local rustic repo of a profile to the source location
    """
    action = Action(name, parallel, "[extract]")
    timer = Timer(parallel)
    profile = Profile(name, parallel)
    profile.read_rustic_config()
    profile.parse_rustic_config()
    profile.set_log_file(log_file)
    profile.check_local_repo_exists()
    profile.latest()
    profile.check_source_type()
    profile.restore()
    timer.stop()
    # print_stats("", "")
    return profile.result, timer.duration
