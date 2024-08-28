"""
Launch RClone binary with passed flags
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of rclone.py                                         │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── CLASSES
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# typing
from typing import Any

# launch binaries
import subprocess

# ################################################################ CLASSES


class Rclone:
    def __init__(self, **kwargs):
        """
        Launch a rclone command
        """
        default_flags = [
            "--auto-confirm",
            "--ask-password=false",
            "--check-first",
            "--cutoff-mode=hard",
            "--delete-during",
            "--fast-list",
            "--links",
            "--human-readable",
            "--stats-one-line",
            "--transfers=10",
            "--verbose",
            "--crypt-server-side-across-configs",
            "--onedrive-server-side-across-configs",
            "--drive-server-side-across-configs",
            "--drive-chunk-size=128M",
            "--drive-acknowledge-abuse",
            "--drive-stop-on-upload-limit",
        ]
        default_kwargs: dict[str, Any] = {
            "env": {},
            "check_return_code": True,
            "action": "version",
            "default_flags": default_flags,
            "additional_flags": [],
            "origin": None,
            "destination": None,
        }
        kwargs = default_kwargs | kwargs
        self.env = kwargs["env"]
        self.check_return_code = kwargs["check_return_code"]
        self.action = kwargs["action"]
        self.origin = kwargs["origin"]
        self.destination = kwargs["destination"]
        if kwargs["default_flags"]:
            self.flags = [
                *kwargs["default_flags"],
                f"--log-file={kwargs['log_file']}",
                # f"--config={kwargs['config']}",
                # f"--password-command=\"echo '{kwargs['config_pass']}'\"",
                *kwargs["additional_flags"],
            ]
        else:
            self.flags = []
        self.command_entries = [
            "rclone",
            *self.flags,
            self.action,
            self.origin,
            self.destination,
        ]
        self.command = [_ for _ in self.command_entries if _ is not None]

        try:
            self.subprocess = subprocess.run(
                self.command,
                check=self.check_return_code,
                capture_output=True,
                env=self.env,
            )
        except FileNotFoundError:
            print("RClone executable not found, are you sure it is installed?")
            self.returncode = 1
        except subprocess.CalledProcessError as exception:
            print("Error env: '" + str(self.env))
            print("Error args: '" + " ".join(self.command) + "'")
            print("Error status: ", exception.returncode)
            print(f'Error stderr:\n{exception.stderr.decode("utf-8")}')
            print("")
            self.returncode = 1
        else:
            self.stdout = self.subprocess.stdout.decode("utf-8")
            self.stderr = self.subprocess.stderr.decode("utf-8")
            self.returncode = self.subprocess.returncode
