"""
Launch Rustic binary with passed flags
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of rustic.py                                         │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── CLASSES
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# launch binaries
import subprocess

# ################################################################ CLASSES


class Rustic:
    def __init__(self, profile: str, action: str, *args: str):
        """
        Launch a rustic command
        """
        self.flags = ["--no-progress"]
        self.command = [
            "rustic",
            *self.flags,
            "--use-profile",
            profile,
            action,
            *args,
        ]
        try:
            # stream output in real time
            # output_list = []
            # with Popen(self.command, parallel=PIPE) as p:
            #    while True:
            #        text = p.stdout.read1().decode("utf-8")
            #        print(text, end='', flush=True)
            #        output_list.append(text)
            # self.stdout = " ".join(output_list)
            # print(self.stdout)
            # wait for completion
            self.subprocess = subprocess.run(
                self.command, check=True, capture_output=True
            )
        except FileNotFoundError:  # pragma: no cover
            print("Rustic executable not found, are you sure it is installed?")
            self.returncode = 1
        except subprocess.CalledProcessError as exception:  # pragma: no cover
            print("Error args: '" + " ".join(self.command) + "'")
            print("Error status: ", exception.returncode)
            print(f'Error stderr:"n{exception.stderr.decode("utf-8")}')
            self.returncode = 1
        else:
            self.stdout = self.subprocess.stdout.decode("utf-8")
            self.stderr = self.subprocess.stderr.decode("utf-8")
            self.returncode = self.subprocess.returncode
