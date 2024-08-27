"""
Define actions that can be run for each Rustic profile
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of profile.py                                        │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── CLASSES
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# types
from typing import Any

# file locations, path concatenation
from pathlib import Path

# stdout parsing
import tomllib
import json
from datetime import datetime

# hostname
import platform

# rusticlone
from rusticlone.helpers.action import Action
from rusticlone.helpers.rclone import Rclone
from rusticlone.helpers.rustic import Rustic
from rusticlone.helpers.formatting import clear_line, print_stats, convert_size

# ################################################################ CLASSES


class Profile:
    """
    Define actions that can be run for each Rustic profile
    """

    def __init__(self, profile: str, parallel: bool = False) -> None:
        """
        Default values for each Rustic profile
        """
        self.profile_name = profile
        self.parallel = parallel
        self.source = ""
        self.repo = ""
        self.log_file = Path("rusticlone.log")
        self.env: dict[str, str] = {}
        self.password_provided = ""
        self.backup_output = ""
        self.source_exists = False
        self.source_type = "dir"
        self.local_repo_exists = False
        self.snapshot_exists = False
        self.result = True
        self.hostname = platform.node()
        self.config: dict[str, Any] = {}
        self.latest_snapshot_timestamp = datetime.min
        # self.repo_info = ""

    def parse_rustic_config(self) -> None:
        """
        Parse the configuration to extract source, repo, log file and environment variables
        """
        if self.result:
            action = Action("Parsing rustic configuration", self.parallel)
            rustic = Rustic(self.profile_name, "show-config")
            try:
                self.config = tomllib.loads(rustic.stdout)
            except tomllib.TOMLDecodeError:
                self.result = action.abort("Could not parse rustic config:")
                print(rustic.stdout)
                print(rustic.stderr)
            else:
                self.result = self.parse_rustic_config_source(action)
                self.result = self.parse_rustic_config_repo(action)
                self.result = self.parse_rustic_config_log(action)
                self.result = self.parse_rustic_config_env(action)
                if self.result:
                    action.stop("Rustic configuration parsed")

    def parse_rustic_config_source(self, action) -> bool:
        """
        Read source from Rustic profile configuration
        """
        try:
            self.source = self.config["backup"]["sources"][0]["source"]
        except KeyError:
            return action.abort("Could not parse source in config:\n", self.config)
        return True

    def parse_rustic_config_repo(self, action) -> bool:
        """
        Read repo from Rustic profile configuration
        """
        try:
            self.repo = self.config["repository"]["repository"]
        except KeyError:
            return action.abort("Could not parse repo in config:\n", self.config)
        return True

    def parse_rustic_config_log(self, action) -> bool:
        """
        Read log file from Rustic profile configuration
        """
        try:
            self.log_file = Path(self.config["global"]["log-file"])
        except KeyError:
            return action.abort(f'Invalid log file: "{str(self.log_file)}"')
        return True

    def parse_rustic_config_env(self, action) -> bool:
        """
        Read environment variables for Rustic and Rclone
        """
        try:
            self.env = self.config["global"]["env"]
        except KeyError:
            return False
        return True

    def check_rclone_config_exists(self) -> None:
        """
        Parse Rustic configuration and extract rclone config, and rclone config password command.
        These will be passed to RClone during upload and download operations, where Rustic is not used.
        """
        if self.result:
            action = Action("Checking Rclone configuration", self.parallel)
            try:
                rclone_config_file = Path(self.env["RCLONE_CONFIG"])
            except (KeyError, TypeError) as exception:
                self.result = action.abort(
                    "Could not parse rclone config:", str(exception)
                )
            else:
                if rclone_config_file.is_file():
                    action.stop("Rclone configuration exists")
                else:
                    self.result = action.abort(
                        f"Rclone configuration file does not exist: {rclone_config_file}"
                    )

    def set_log_file(self, passed_log_file: Path) -> None:
        """
        set rclone log file
        if not default has been passed, use it, otherwise use the one in conf
        if there are no matches in conf, use the default one
        rustic fails if it cannot find the parent folder of the log file when parsing the conf,
        so it must be created before Profile.read_rustic_config() is run
        from now on, use self.log_file in Profile.upload() and Profile.download()
        """
        if self.result:
            action = Action("Setting log file", self.parallel)
            if passed_log_file != Path("rusticlone.log"):
                self.log_file = passed_log_file
            if self.parallel:
                suffix_old = self.log_file.suffix
                suffix_new = "-" + self.profile_name + ".log"
                self.log_file = Path(str(self.log_file).replace(suffix_old, suffix_new))
                # rustic fails anyway if it cannot find the path when parsing the conf
                # self.log_file.parent.mkdir(parents=True, exist_ok=True)
                # self.log_file.touch(exist_ok=True)
            action.stop("Log file set")

    def check_source_exists(self) -> None:
        """
        Check if the source exists and handle accordingly.
        """
        if self.result:
            action = Action("Checking if source exists", self.parallel)
            # print(self.source)
            source = Path(self.source)
            if source.exists():
                if source.is_dir():
                    self.source_type = "dir"
                else:
                    self.source_type = "file"
                self.source_exists = True
                action.stop("Source exists")
            else:
                # self.source_exists = False
                self.result = action.abort("Source does not exist")

    def check_local_repo_exists(self) -> None:
        """
        Check local repo folder with pathlib
        """
        if self.result:
            action = Action("Checking if local repo exists", self.parallel)
            # self.repo_type = "local"
            repo_config_file = Path(self.repo) / "config"
            if repo_config_file.exists() and repo_config_file.is_file():
                self.local_repo_exists = True
                action.stop("Local repo exists")
            else:
                self.local_repo_exists = False
                action.stop("Local repo does not exist")

    def check_remote_repo_exists(self, remote_prefix: str) -> None:
        """
        Check remote repo folder with rclone
        """
        if self.result:
            action = Action("Checking if remote repo exists", self.parallel)
            rclone_log_file = str(self.log_file)
            # rclone_origin = remote_prefix + "/" + self.profile_name
            repo_name = str(Path(self.repo).name)
            rclone_origin = remote_prefix + "/" + repo_name
            rclone_lsd = Rclone(
                env=self.env,
                log_file=rclone_log_file,
                action="lsd",
                origin=rclone_origin,
                check_return_code=False,
            )
            # == 3 if does not exist
            if rclone_lsd.returncode == 0:
                action.stop("Remote repo exists")
            else:
                self.result = action.abort(
                    f"Remote repo does not exist, Rclone exit code: {rclone_lsd.returncode}"
                )

    def init(self) -> None:
        """
        Initialize the repository if it does not exist, and perform the necessary setup.
        This method does not take any parameters and returns None, however it needs to know if
        the local repo already exists
        """
        # if self.local_repo_exists:
        #    action = Action("Using existing repo", self.parallel)
        # else:
        if self.result:
            if not self.local_repo_exists:
                action = Action("Initializing new local repo", self.parallel)
                # will go interactive if [repository] password is not set
                rustic = Rustic(
                    self.profile_name,
                    "init",
                    "--set-treepack-size",
                    "50MB",
                    "--set-datapack-size",
                    "500MB",
                    "--log-file",
                    str(self.log_file),
                )
                self.local_repo_exists = True
                action.stop("Initialized new local repo")

    def check_local_repo_health(self) -> None:
        """
        Only check local repos for speed purposes
        """
        if self.result:
            if self.local_repo_exists:
                action = Action("Checking repo health", self.parallel)
                Rustic(self.profile_name, "check", "--log-file", str(self.log_file))
                action.stop("Repo is healthy")

    def backup(self):
        """
        Perform a backup operation and store the output in self.backup_output.
        """
        if self.result:
            if self.source_exists:
                action = Action("Backing up source", self.parallel)
                rustic = Rustic(
                    self.profile_name,
                    "backup",
                    "--json",
                    "--log-file",
                    str(self.log_file),
                )
                action.stop("Snapshot created")
                self.backup_output = rustic.stdout
                # print(self.backup_output)

    def source_stats(self) -> None:
        """
        A method to gather source statistics and print them to the console.
        This method does not take any parameters and does not return anything.
        """
        if self.result:
            # action = Action("Retrieving source stats", self.parallel)
            action = Action("Retrieving stats", self.parallel)
            # print(self.backup_output)
            # print(self.backup_output)
            if self.source_exists and self.backup_output != "":
                json_output = json.loads(self.backup_output)
                # print(json_output)
                source_files = json_output["summary"]["total_files_processed"]
                source_size = json_output["summary"]["total_bytes_processed"]
                snapshot_size = json_output["summary"]["data_added"]
                # else:
                #    source_files = 0
                #    source_size = 0
                #    snapshot_size = 0
                clear_line(parallel=self.parallel)
                # action.stop("Retrieved source stats")
                print_stats(
                    "Source size:", convert_size(source_size), parallel=self.parallel
                )
                print_stats("Source files:", source_files, parallel=self.parallel)
                print_stats(
                    "Snapshot size:",
                    convert_size(snapshot_size),
                    parallel=self.parallel,
                )
                print_stats("", "", parallel=self.parallel)
            else:
                self.result = False  # action.abort("Empty backup output")

    def repo_stats(self) -> None:
        """
        Get repo stats and print the number of files and the total size of the repository.
        """
        if self.result:
            if self.local_repo_exists:
                # action = Action("Retrieving repo stats", self.parallel)
                # saving one command, even if it's run before snapshot
                # json_output = json.loads(self.repo_info)
                rustic = Rustic(
                    self.profile_name,
                    "repoinfo",
                    "--json",
                    "--log-file",
                    str(self.log_file),
                )
                if rustic.stdout != "":
                    json_output = json.loads(rustic.stdout)
                    # print(json_output)
                    # "config" is not included in repoinfo
                    repo_files = 1
                    repo_size = 0
                    repo_files += sum(
                        [entry["count"] for entry in json_output["files"]["repo"]]
                    )
                    repo_size += sum(
                        [entry["size"] for entry in json_output["files"]["repo"]]
                    )
                    clear_line(parallel=self.parallel)
                    # action.stop("Retrieved repo stats")
                    print_stats(
                        "Repo size:", convert_size(repo_size), parallel=self.parallel
                    )
                    print_stats("Repo files:", str(repo_files), parallel=self.parallel)
                else:
                    self.result = False  # action.abort("Repoinfo output is empty")

    def forget(self) -> None:
        """
        A method to perform the action of forgetting, with no parameters and returning None.
        """
        if self.result:
            if self.backup_output != "":
                action = Action("Pruning repo", self.parallel)
                Rustic(
                    self.profile_name,
                    "forget",
                    "--json",
                    "--fast-repack",
                    "--keep-last",
                    "1",
                    "--log-file",
                    str(self.log_file),
                )
                action.stop("Repo pruned")

    def upload(self, remote_prefix: str) -> None:
        """
        Uploads the local repository to a remote destination using rclone.
        """
        if self.result:
            action = Action("Uploading repo", self.parallel)
            rclone_log_file = str(self.log_file)
            rclone_origin = self.repo.replace("\\", "/").replace("//", "/")
            # rclone_destination = remote_prefix + "/" + self.profile_name
            repo_name = str(Path(self.repo).name)
            rclone_destination = remote_prefix + "/" + repo_name
            # print(rclone_destination)
            rclone_upload = Rclone(
                env=self.env,
                log_file=rclone_log_file,
                action="sync",
                # 1.67+, if added to 1.65.2 complains that log_file is an invalid option
                other_flags=[
                    "--create-empty-src-dirs=false",
                    "--no-update-dir-modtime",
                ],
                origin=rclone_origin,
                destination=rclone_destination,
            )
            # action.stop(f"Uploaded repo: {rclone_destination}")
            if rclone_upload.returncode == 0:
                action.stop("Uploaded repo")
            else:
                self.result = action.abort("Error uploading repo")

    def download(self, remote_prefix: str) -> None:
        """
        Uploads the remote repository to a local destination using rclone.
        """
        if self.result:
            if not self.repo.startswith("rclone:"):
                action = Action("Downloading repo", self.parallel)
                if not self.local_repo_exists:
                    rclone_log_file = str(self.log_file)
                    # rclone_origin = remote_prefix + "/" + self.profile_name
                    repo_name = str(Path(self.repo).name)
                    rclone_origin = remote_prefix + "/" + repo_name
                    rclone_destination = self.repo
                    Rclone(
                        env=self.env,
                        log_file=rclone_log_file,
                        action="sync",
                        other_flags=[
                            "--create-empty-src-dirs=false",
                            "--no-update-dir-modtime",
                        ],
                        origin=rclone_origin,
                        destination=rclone_destination,
                    )
                    action.stop("Downloaded repo")
                    # action.stop(f"Downloaded repo: {rclone_destination}")
                else:
                    action.stop("Repo already downloaded")
                # print(self.repo)
        # else:
        #    action.stop("Not downloading remote-only repo, extracting it directly")

    def check_latest_snapshot(self) -> None:
        """
        Read the latest snapshot from a local repo that has just been downloaded
        Require local repo and snapshot to exist
        """
        if self.result:
            action = Action("Retrieving latest snapshot", self.parallel)
            if self.local_repo_exists:
                rustic = Rustic(
                    self.profile_name,
                    "snapshots",
                    "latest",
                    "--json",
                    "--log-file",
                    str(self.log_file),
                )
                json_output = json.loads(rustic.stdout)
                if json_output is not None and len(json_output) > 0:
                    # print(f"output: {json_output}"
                    try:
                        self.latest_snapshot_timestamp = datetime.fromisoformat(
                            json_output[-1][1][0]["time"]
                        )
                    except ValueError:
                        self.result = action.abort("Could not parse timestamp")
                    else:
                        timestamp_pretty = self.latest_snapshot_timestamp.strftime(
                            "%Y-%m-%d %H:%M:%S"
                        )
                        clear_line(parallel=self.parallel)
                        # self.snapshot_exists = True
                        print_stats(
                            "Restoring from:",
                            f"{timestamp_pretty}",
                            20,
                            20,
                            parallel=self.parallel,
                        )
                else:
                    self.result = action.abort("Repo does not have snapshots")
            else:
                self.result = action.abort("Local repo does not exist")

    def check_source_type(self) -> None:
        """
        Check if the source that needs to be restored is a directory or file
        Require local repo and snapshot to exist
        """
        if self.result:
            action = Action("Checking source type", self.parallel)
            rustic = Rustic(
                self.profile_name,
                "ls",
                "latest",
                "--glob",
                f"{self.source}",
                "--long",
                "--log-file",
                str(self.log_file),
            )
            if rustic.stdout[0] == "d" or rustic.stdout.count("\n") > 1:
                self.source_type = "dir"
            else:
                self.source_type = "file"
            action.stop(f"Source is a {self.source_type}")

    def restore(self) -> None:
        """
        Extract files in the latest snapshot to the source location, after creating it if missing
        if self.source is a directory, it is created if missing
        if self.source is a file, its parent is created if missing
        Require local repo and snapshot to exist
        """
        if self.result:
            action = Action("Extracting snapshot", self.parallel)
            if self.source_type == "dir":
                Path(self.source).mkdir(parents=True, exist_ok=True)
            else:
                Path(self.source).parent.mkdir(parents=True, exist_ok=True)
            Rustic(
                self.profile_name,
                "restore",
                f"latest:{self.source}",
                self.source,
                "--log-file",
                str(self.log_file),
            )
            action.stop("Snapshot extracted")
