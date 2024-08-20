"""
Load customizations by interpreting passed arguments
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of custom.py                                         │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── CLASSES
# ├── FUNCTIONS
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# file locations
from pathlib import Path

# args type
from argparse import Namespace

# os and hostname
import platform

# exit
import sys

# rusticlone
from rusticlone.helpers.action import Action
from rusticlone.processing.parallel import (
    system_backup_parallel,
    system_archive_parallel,
    system_upload_parallel,
    system_restore_parallel,
    system_download_parallel,
    system_extract_parallel,
)
from rusticlone.processing.sequential import (
    system_backup_sequential,
    system_archive_sequential,
    system_upload_sequential,
    system_restore_sequential,
    system_download_sequential,
    system_extract_sequential,
)

# ################################################################ CLASSES


class Custom:
    """
    Set variables to args flags, or use hardcoded values
    """

    def __init__(self, args) -> None:
        """
        Initialize variables
        """
        self.hostname = platform.node()
        self.parallel = args.parallel
        self.command = args.command[0]
        self.operating_system = platform.system()
        self.default_log_file = Path("rusticlone.log")
        # log file
        # rustic use log file from config, while from rclone it is passed from either cli or here
        if args.log_file is not None:
            self.log_file = Path(args.log_file)
        else:
            self.log_file = self.default_log_file
        # profiles dirs, the same where rustic reads profiles
        if self.operating_system == "Windows":
            self.profiles_dirs = [
                Path.home() / "AppData/Roaming/rustic/config",
                Path("C:/ProgramData/rustic/config"),
            ]
        else:
            self.profiles_dirs = [
                Path.home() / ".config/rustic",
                Path("/etc/rustic"),
            ]
        # remote prefix: rclone remote + subdirectory without trailing slash
        if args.remote is not None:
            self.remote_prefix = args.remote.rstrip("/")
        else:
            self.remote_prefix = "None:/"
        # ignore pattern for profiles
        self.ignore_pattern = args.ignore
        # test profile
        if args.profile:
            self.profiles = args.profile
        else:
            self.profiles = []

    def check_log_file(self):
        """
        Create log file parent folders if missing, delete old log file
        """
        action = Action("Checking log file")
        if self.log_file != self.default_log_file:
            used_log = self.log_file
            self.log_file.parents[0].mkdir(parents=True, exist_ok=True)
            self.log_file.unlink(missing_ok=True)
            self.log_file.touch()
        else:
            # if not defined in Rustic profiles, "./rusticlone.log" is used
            used_log = "defined in Rustic profiles"
        action.stop(f"Log file: {used_log}", "")


# ################################################################ FUNCTIONS


def list_profiles(
    profiles_dirs: list, profiles: list, ignore_pattern: str = "🫣🫣🫣"
) -> list:
    """
    Scan profiles from directories if none have been provided explicitely
    Don't scan from /etc/rustic if ~/.config/rustic has some profiles'
    """
    action = Action("Reading profiles")
    if not profiles:
        for profiles_dir in profiles_dirs:
            if len(profiles) == 0 and profiles_dir.exists() and profiles_dir.is_dir():
                action.stop(f'Scanning "{profiles_dir}"', "")
                files = sorted(list(profiles_dir.glob("*.toml")))
                for file in files:
                    if (
                        file.is_file()
                        and ignore_pattern not in file.stem
                        and file.stem not in profiles
                    ):
                        profiles.append(file.stem)
    if len(profiles) == 0:
        action.abort("Could not find any rustic profile")
        sys.exit(1)
    else:
        action.stop(f"Profiles: {str(profiles)}", "")
        return profiles


def process_profiles(
    profiles: list,
    parallel: bool,
    command: str,
    log_file: Path,
    remote_prefix: str,
) -> None:
    """
    Process all profiles according to the command specified and parallel flag
    """
    if parallel:
        match command:
            case "backup":
                system_backup_parallel(
                    profiles=profiles, log_file=log_file, remote_prefix=remote_prefix
                )
            case "archive":
                system_archive_parallel(profiles=profiles, log_file=log_file)
            case "upload":
                system_upload_parallel(
                    profiles=profiles, log_file=log_file, remote_prefix=remote_prefix
                )
            case "restore":
                system_restore_parallel(
                    profiles=profiles,
                    log_file=log_file,
                    remote_prefix=remote_prefix,
                )
            case "download":
                system_download_parallel(
                    profiles=profiles, log_file=log_file, remote_prefix=remote_prefix
                )
            case "extract":
                system_extract_parallel(profiles=profiles, log_file=log_file)
    else:
        match command:
            case "backup":
                system_backup_sequential(
                    profiles=profiles, log_file=log_file, remote_prefix=remote_prefix
                )
            case "archive":
                system_archive_sequential(profiles=profiles, log_file=log_file)
            case "upload":
                system_upload_sequential(
                    profiles=profiles, log_file=log_file, remote_prefix=remote_prefix
                )
            case "restore":
                system_restore_sequential(
                    profiles=profiles,
                    log_file=log_file,
                    remote_prefix=remote_prefix,
                )
            case "download":
                system_download_sequential(
                    profiles=profiles, log_file=log_file, remote_prefix=remote_prefix
                )
            case "extract":
                system_extract_sequential(profiles=profiles, log_file=log_file)


def load_customizations(args: Namespace):
    """
    Interpret arguments, make a list of profiles to process, and process them
    """
    custom = Custom(args)
    custom.check_log_file()
    profiles = list_profiles(
        custom.profiles_dirs, custom.profiles, custom.ignore_pattern
    )
    process_profiles(
        profiles,
        custom.parallel,
        custom.command,
        custom.log_file,
        custom.remote_prefix,
    )
