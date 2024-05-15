"""
Process all the Rustic profiles in the system at the same time,
but archiving before uploading and downloading before extracting
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of parallel.py                                       │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├──┐FUNCTIONS
# │  ├── GENERIC
# │  ├── BACKUP
# │  └── RESTORE
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# file locations
from pathlib import Path

# multithreading
import concurrent.futures

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
# ################################ GENERIC


def duration_parallel(processed_profiles: dict, command: str) -> None:
    """
    Print duration for each atomic operation
    Cannot use inside system_upload_parallel() and system_extract_parallel()
    because they also launch a function if success
    """
    for future in concurrent.futures.as_completed(processed_profiles):
        name = processed_profiles[future]
        try:
            success, duration = future.result()
        except Exception as exception:
            print_stats(f"Failure {command} {name}: '{exception}'", "[KO]")
        else:
            if success:
                print_stats(f"{command} {name}", duration)
            else:
                print_stats(f"Failure {command} {name}", "[KO]")


# ################################ BACKUP


def system_backup_parallel(profiles: list, log_file: Path, remote_prefix: str) -> None:
    """
    start a ThreadPoolExecutor and launch system_archive_parallel()
    Once a profile has a been archived, upload it without waiting for others
    """
    # action = Action("Backing up system", status="")
    action = Action("System", status="[backup]")
    timer = Timer()
    with concurrent.futures.ThreadPoolExecutor(
        thread_name_prefix="SystemBackup"
    ) as executor:
        archived_profiles = system_archive_parallel(
            profiles=profiles,
            log_file=log_file,
            executor=executor,
        )
        uploaded_profiles = system_upload_parallel(
            profiles=profiles,
            log_file=log_file,
            remote_prefix=remote_prefix,
            archived_profiles=archived_profiles,
            executor=executor,
        )
        duration_parallel(uploaded_profiles, "Uploading")
    # print(uploaded_profiles)
    # print("DONE!")
    timer.stop()


def system_archive_parallel(profiles: list, log_file: Path, executor=None) -> dict:
    """
    if launched independently ,start a ThreadPoolExecutor
    For every profile, archive it
    """
    archived_profiles = {}
    # reuse executor or start a new one if launched independently
    if executor is not None:
        archived_profiles = {
            executor.submit(
                profile_archive, name=name, log_file=log_file, parallel=True
            ): name
            for name in profiles
        }
    else:
        with concurrent.futures.ThreadPoolExecutor(
            thread_name_prefix="SystemArchiveOnly"
        ) as executor:
            action = Action("System", status="[archive]")
            archived_profiles = {
                executor.submit(
                    profile_archive, name=name, log_file=log_file, parallel=True
                ): name
                for name in profiles
            }
            duration_parallel(archived_profiles, "Archiving")
    return archived_profiles


def system_upload_parallel(
    profiles: list,
    log_file: Path,
    remote_prefix: str,
    archived_profiles: dict = {},
    executor=None,
) -> dict:
    """
    if launched independently ,start a ThreadPoolExecutor
    otherwise check that the archival operation completed without errors
    For every profile, upload it it
    """
    uploaded_profiles = {}
    if archived_profiles and executor is not None:
        for future in concurrent.futures.as_completed(archived_profiles):
            try:
                success, duration = future.result()
                name = archived_profiles[future]
            except Exception as exception:
                print(f"Failure in archiving: '{exception}'")
            else:
                if success:
                    uploaded_profiles[
                        executor.submit(
                            profile_upload,
                            name=name,
                            log_file=log_file,
                            remote_prefix=remote_prefix,
                            parallel=True,
                        )
                    ] = name
                    print_stats(f"Archiving {name}", duration)
                else:
                    print(f"Not uploading {name} due to failure in archiving")
    else:
        with concurrent.futures.ThreadPoolExecutor(
            thread_name_prefix="SystemUploadOnly"
        ) as executor:
            action = Action("System", status="[upload]")
            uploaded_profiles = {
                executor.submit(
                    profile_upload,
                    name=name,
                    log_file=log_file,
                    remote_prefix=remote_prefix,
                    parallel=True,
                ): name
                for name in profiles
            }
    return uploaded_profiles


# ################################ RESTORE


def system_restore_parallel(profiles: list, log_file: Path, remote_prefix: str) -> None:
    """
    start a ThreadPoolExecutor and launch system_download_parallel()
    Once a profile has a been downloaded, extract it without waiting for others
    """
    # action = Action("Restoring system", status="")
    action = Action("System", status="[restore]")
    timer = Timer()
    with concurrent.futures.ThreadPoolExecutor(
        thread_name_prefix="SystemRestore"
    ) as executor:
        downloaded_profiles = system_download_parallel(
            profiles=profiles,
            log_file=log_file,
            remote_prefix=remote_prefix,
            executor=executor,
        )
        extracted_profiles = system_extract_parallel(
            profiles=profiles,
            log_file=log_file,
            downloaded_profiles=downloaded_profiles,
            executor=executor,
        )
        duration_parallel(extracted_profiles, "Extracting")
    timer.stop()


def system_download_parallel(
    profiles: list,
    log_file: Path,
    remote_prefix: str,
    executor=None,
) -> dict:
    """
    if launched independently ,start a ThreadPoolExecutor
    For every profile, download it
    """
    downloaded_profiles = {}
    # reuse executor or start a new one if launched independently
    if executor is not None:
        downloaded_profiles = {
            executor.submit(
                profile_download,
                name=name,
                log_file=log_file,
                remote_prefix=remote_prefix,
                parallel=True,
            ): name
            for name in profiles
        }
    else:
        with concurrent.futures.ThreadPoolExecutor(
            thread_name_prefix="SystemDownloadOnly"
        ) as executor:
            action = Action("System", status="[download]")
            downloaded_profiles = {
                executor.submit(
                    profile_download,
                    name=name,
                    log_file=log_file,
                    remote_prefix=remote_prefix,
                    parallel=True,
                ): name
                for name in profiles
            }
            duration_parallel(downloaded_profiles, "Downloading")
    return downloaded_profiles


def system_extract_parallel(
    profiles: list,
    log_file: Path,
    downloaded_profiles: dict = {},
    executor=None,
) -> dict:
    """
    if launched independently ,start a ThreadPoolExecutor
    otherwise check that the download operation completed without errors
    For every profile, extract it it
    """
    extracted_profiles = {}
    if downloaded_profiles and executor is not None:
        for future in concurrent.futures.as_completed(downloaded_profiles):
            try:
                success, duration = future.result()
                name = downloaded_profiles[future]
            except Exception as exception:
                print(f"Failure in download: '{exception}'")
            else:
                if success:
                    extracted_profiles[
                        executor.submit(
                            profile_extract,
                            name=name,
                            log_file=log_file,
                            parallel=True,
                        )
                    ] = name
                    print_stats(f"Downloading {name}", duration)
                else:
                    print(f"Not extracting {name} due to failure in download")
    else:
        with concurrent.futures.ThreadPoolExecutor(
            thread_name_prefix="SystemExtractOnly"
        ) as executor:
            action = Action("System", status="[extract]")
            extracted_profiles = {
                executor.submit(
                    profile_extract,
                    name=name,
                    log_file=log_file,
                    parallel=True,
                ): name
                for name in profiles
            }
    return extracted_profiles
