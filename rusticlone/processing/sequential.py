"""
Process all the Rustic profiles in the system one at the time
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of sequential.py                                     │
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
from rusticlone.helpers.formatting import print_stats
from rusticlone.processing.atomic import (
    profile_archive,
    profile_upload,
    profile_download,
    profile_extract,
)

# ################################################################ FUNCTIONS
# ################################ BACKUP


def system_backup_sequential(
    profiles: list, log_file: Path, remote_prefix: str
) -> None:
    """
    Launch system_archive() and system_upload()
    """
    # action = Action("Backing up system", status="")
    print("========================================")
    action = Action("System", status="[backup]")
    timer = Timer()
    archive_results = system_archive_sequential(profiles=profiles, log_file=log_file)
    system_upload_sequential(
        profiles=profiles,
        log_file=log_file,
        remote_prefix=remote_prefix,
        archive_results=archive_results,
    )
    timer.stop()


def system_archive_sequential(profiles: list, log_file: Path) -> dict:
    """
    For each profile, archive it

    Args:
        profiles (list): List of profiles to be archived

    Returns:
        None
    """
    # print_stats("Creating local snapshots", "")
    print("________________________________________")
    print_stats("System", "[archive]")
    print_stats("", "")
    archive_results = {}
    for name in profiles:
        archive_success, duration = profile_archive(name=name, log_file=log_file)
        archive_results[name] = archive_success
        if archive_success:
            print_stats("", "")
        else:
            print_stats(f"Error archiving {name}", "")
            print_stats("", "")
    return archive_results


def system_upload_sequential(
    profiles: list,
    log_file: Path,
    remote_prefix: str,
    archive_results: dict | None = None,
) -> None:
    """
    For each profile, upload it

    Args:
        profiles (list): List of profiles to be uploaded
        log_file (Path): log file for Rclone
        remote_prefix (str): prefix of Rclone remote

    Returns:
        None
    """
    # print_stats("Uploading local snapshots", "")
    print("________________________________________")
    print_stats("System", "[upload]")
    print_stats("", "")
    if archive_results is None:
        archive_results = {}
    for name in profiles:
        archive_success = archive_results.get(name, True)
        if archive_success:
            upload_success, duration = profile_upload(
                name=name, log_file=log_file, remote_prefix=remote_prefix
            )
            if upload_success:
                print_stats("", "")
            else:
                print_stats(f"Error uploading {name}", "")
        else:
            print_stats(f"Not uploading {name} as archiving failed", "")
            print_stats("", "")


# ################################ RESTORE


def system_restore_sequential(
    profiles: list, log_file: Path, remote_prefix: str
) -> None:
    """
    Launch system_download() and system_extract()
    """
    # action = Action("Restoring system", status="")
    print("========================================")
    action = Action("System", False, "[restore]")
    timer = Timer()
    download_results = system_download_sequential(
        profiles, log_file=log_file, remote_prefix=remote_prefix
    )
    system_extract_sequential(
        profiles=profiles,
        log_file=log_file,
        download_results=download_results,
    )
    timer.stop()


def system_download_sequential(
    profiles: list, log_file: Path, remote_prefix: str
) -> dict:
    """
    For each profile, download it to the repo location

    Args:
        profiles (list): List of profiles to be uploaded
        log_file (Path): log file for Rclone
        remote_prefix (str): prefix of Rclone remote

    Returns:
        None
    """
    # print_stats("Downloading remote snapshots", "")
    print("________________________________________")
    print_stats("System", "[download]")
    print_stats("", "")
    download_results = {}
    for name in profiles:
        download_success, duration = profile_download(
            name=name, log_file=log_file, remote_prefix=remote_prefix
        )
        download_results[name] = download_success
        if download_success:
            print_stats("", "")
        else:
            print_stats(f"Error downloading {name}", "")
    return download_results


def system_extract_sequential(
    profiles: list,
    log_file: Path,
    download_results: dict | None = None,
) -> None:
    """
    For each profile,
    extract it to a specific folder,
    move the extracted snapshot to root location
    """
    # print_stats("Extracting local snapshots", "")
    print("________________________________________")
    print_stats("System", "[extract]")
    print_stats("", "")
    if download_results is None:
        download_results = {}
    for name in profiles:
        download_success = download_results.get(name, True)
        if download_success:
            extract_success, duration = profile_extract(name=name, log_file=log_file)
            if extract_success:
                print_stats("", "")
            else:
                print_stats(f"Error extracting {name}", "")
        else:
            print_stats(f"Not extracting {name} as download failed", "")
