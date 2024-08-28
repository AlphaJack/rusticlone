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
        self.repo = ""
        self.log_file = Path("rusticlone.log")
        self.env: dict[str, str] = {}
        self.password_provided = ""
        # json objects
        self.backup_output: list[dict[Any, Any]] = []
        self.sources: list[str] = []
        self.sources_exist: dict[str, bool] = {}
        self.sources_type: dict[str, str] = {}
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
            except (AttributeError, tomllib.TOMLDecodeError):
                self.result = action.abort("Could not parse rustic configuration")
            else:
                self.result = self.parse_rustic_config_source(action)
                self.result = self.parse_rustic_config_repo(action)
                self.result = self.parse_rustic_config_log(action)
                self.result = self.parse_rustic_config_env(action)
                if self.result:
                    action.stop("Parsed rustic configuration")

    def parse_rustic_config_source(self, action) -> bool:
        """
        Read sources from Rustic profile configuration
        """
        try:
            # self.source = self.config["backup"]["sources"][0]["source"]
            # they can be either string or list of strings:
            # https://github.com/rustic-rs/rustic/blob/a88afdd4af295c16e5de50de91ec430920f81f56/config/full.toml
            config_sources = [
                source["source"] for source in self.config["backup"]["sources"]
            ]
            for config_source in config_sources:
                if config_source and isinstance(config_source, list):
                    self.sources.extend(config_source)
                elif config_source:
                    self.sources.append(config_source)
            # remove eventual duplicates
            self.sources = list(set(self.sources))
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
            return True
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
            action.stop("Set log file")

    def check_sources_exist(self) -> None:
        """
        Check if all the file sources for the profile exist in the local filesystem
        """
        if self.result:
            action = Action("Checking if sources exists", self.parallel)
            # print(self.source)
            for source in self.sources:
                source_path = Path(source)
                if source_path.exists():
                    self.sources_exist[source] = True
                else:
                    self.sources_exist[source] = False
            if all(self.sources_exist.values()):
                if len(self.sources) > 1:
                    plural_form = "sources"
                else:
                    plural_form = "source"
                action.stop(f"Found {len(self.sources)} {plural_form}")
            else:
                self.result = action.abort("Some sources do not exist")

    def check_local_repo_exists(self) -> None:
        """
        Check if the local repo folder exists using pathlib
        The program doesn't fail if the local repo doesn't exist,
        because we can create it with rustic init or by downloading it
        However, some restore functions depend on its existence,
        that's why we store the check result as a boolean variable.

        Skip if missing local repo:
            - check_local_repo_health(), init(), download()

        Fail if missing local repo:
            - repo_stats(), check_latest_snapshot(), check_source_type(), restore()
        """
        if self.result:
            action = Action("Checking if local repo exists", self.parallel)
            # self.repo_type = "local"
            repo_config_file = Path(self.repo) / "config"
            if repo_config_file.exists() and repo_config_file.is_file():
                self.local_repo_exists = True
                action.stop("Local repo already exists")
            else:
                self.local_repo_exists = False
                action.stop("Local repo does not exist yet")

    def check_local_repo_health(self) -> None:
        """
        Only check local repos for speed purposes
        """
        if self.result:
            if self.local_repo_exists:
                action = Action("Checking repo health", self.parallel)
                Rustic(self.profile_name, "check", "--log-file", str(self.log_file))
                action.stop("Repo is healthy")

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
            rclone = Rclone(
                env=self.env,
                log_file=rclone_log_file,
                action="lsd",
                origin=rclone_origin,
                check_return_code=False,
            )
            # == 3 if does not exist
            if rclone.returncode != 0:
                self.result = action.abort(
                    f"Remote repo does not exist, Rclone exit code: {rclone.returncode}"
                )
            else:
                action.stop("Remote repo exists")

    def init(self) -> None:
        """
        Initialize the repository if it does not exist, and perform the necessary setup.
        This method does not take any parameters and returns None, however it needs to know if
        the local repo already exists.
        We don't remove this function even if "--init" flag in backup would do the trick,
        as we set custom treepack and datapack sizes
        """
        # if self.local_repo_exists:
        #    action = Action("Using existing repo", self.parallel)
        # else:
        if self.result:
            if not self.local_repo_exists:
                action = Action("Initializing a new local repo", self.parallel)
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
                if rustic.returncode != 0:
                    self.result = action.abort("Could not inizialize a new local repo")
                else:
                    self.local_repo_exists = True
                    action.stop("Initialized a new local repo")

    def backup(self) -> None:
        """
        Perform a backup operation and store the output in self.backup_output.
        If multiple sources per profile are set, the output is the concatenation of json objects
        https://stackoverflow.com/a/42985887
        """
        if self.result:
            action = Action("Creating snapshot", self.parallel)
            rustic = Rustic(
                self.profile_name,
                "backup",
                "--init",
                "--json",
                "--log-file",
                str(self.log_file),
            )
            try:
                text = rustic.stdout.lstrip()
                while text:
                    json_object, index = json.JSONDecoder().raw_decode(text)
                    text = text[index:].lstrip()
                    self.backup_output.append(json_object)
            except (AttributeError, json.JSONDecodeError):
                # print(json.loads(rustic.stdout))
                self.result = action.abort("Could not create snapshot")
            else:
                action.stop("Created Snapshot")

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
            source_files = 0
            source_size = 0
            snapshot_size = 0
            try:
                for json_object in self.backup_output:
                    summary = json_object["summary"]
                    source_files += int(summary["total_files_processed"])
                    source_size += int(summary["total_bytes_processed"])
                    snapshot_size += int(summary["data_added"])
            # else:
            #    source_files = 0
            #    source_size = 0
            #    snapshot_size = 0
            except (KeyError, TypeError):
                self.result = action.abort("Could not retrieve stats")
            else:
                clear_line(parallel=self.parallel)
                # action.stop("Retrieved source stats")
                print_stats(
                    "Number of sources:",
                    str(len(self.backup_output)),
                    parallel=self.parallel,
                )
                print_stats(
                    "Files in sources:", str(source_files), parallel=self.parallel
                )
                print_stats(
                    "Size of sources:",
                    convert_size(source_size),
                    parallel=self.parallel,
                )
                print_stats(
                    "Size of snapshot:",
                    convert_size(snapshot_size),
                    parallel=self.parallel,
                )
                print_stats("", "", parallel=self.parallel)

    def repo_stats(self) -> None:
        """
        Get repo stats and print the number of files and the total size of the repository.
        """
        if self.result:
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
                    "Size of repo:", convert_size(repo_size), parallel=self.parallel
                )
                print_stats("Files in repo:", str(repo_files), parallel=self.parallel)
            else:
                self.result = False  # action.abort("Repoinfo output is empty")

    def forget(self) -> None:
        """
        A method to perform the action of forgetting, with no parameters and returning None.
        """
        if self.result:
            action = Action("Deprecating old snapshots", self.parallel)
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
            action.stop("Deprecated old snapshots")

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
            rclone = Rclone(
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
            if rclone.returncode != 0:
                self.result = action.abort("Could not upload repo")
            else:
                action.stop("Uploaded repo")

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
                            f"[{timestamp_pretty}]",
                            19,
                            21,
                            parallel=self.parallel,
                        )
                else:
                    self.result = action.abort("Repo does not have snapshots")
            else:
                self.result = action.abort("Local repo does not exist")

    def check_sources_type(self) -> None:
        """
        Check if the source that needs to be restored is a directory or file
        Require local repo and snapshot to exist
        """
        if self.result:
            action = Action("Checking type of sources", self.parallel)
            for source in self.sources:
                rustic = Rustic(
                    self.profile_name,
                    "ls",
                    "latest",
                    "--filter-paths",
                    f"{source}",
                    "--glob",
                    f"{source}",
                    "--long",
                    "--log-file",
                    str(self.log_file),
                )
                try:
                    rustic.stdout[0]
                except IndexError:
                    # empty folders do not return results
                    pass
                except AttributeError:
                    # error in command
                    self.result = action.abort("Could not determine type of source")
                else:
                    if rustic.stdout[0] == "d" or rustic.stdout.count("\n") > 1:
                        self.sources_type[source] = "dir"
                    else:
                        self.sources_type[source] = "file"
            action.stop("Stored source types")

    def restore(self) -> None:
        """
        Extract files in the latest snapshot to the source location, after creating it if missing
        if source is a directory, it is created if missing
        if source is a file, its parent is created if missing
        Require local repo and snapshot to exist
        """
        if self.result:
            action = Action("Extracting snapshot", self.parallel)
            for source, source_type in self.sources_type.items():
                if self.result:
                    if source_type == "dir":
                        Path(source).mkdir(parents=True, exist_ok=True)
                    else:
                        Path(source).parent.mkdir(parents=True, exist_ok=True)
                    rustic = Rustic(
                        self.profile_name,
                        "restore",
                        f"latest:{source}",
                        source,
                        "--filter-paths",
                        f"{source}",
                        "--log-file",
                        str(self.log_file),
                    )
                    if rustic.returncode != 0:
                        self.result = action.abort(f"Error extracting '{source}'")
            action.stop("Snapshot extracted")
