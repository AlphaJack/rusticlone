"""
Print the name of the operation, and is called again in case of success or failure
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of action.py                                         │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── CLASSES
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# rusticlone
from rusticlone.helpers.formatting import clear_line, print_stats

# ################################################################ CLASSES


class Action:
    def __init__(
        self,
        name_start: str,
        parallel: bool = False,
        status: str = "[W8]",
    ) -> None:
        """
        Initializes the instance with the provided action_start parameter.

        Parameters:
            action_start (str): The action to be assigned to the instance.

        Returns:
            None
        """
        self.name = name_start
        self.parallel = parallel
        # print(self.action)
        # print_stats(self.action, f'[{blink("W8")}]')
        print_stats(self.name, status, parallel=self.parallel)

    def stop(self, name_stop: str = "", status: str = "[OK]") -> bool:
        """
        Stop the action and print the status as OK.

        Args:
            action_stop (str): The action to stop.

        Returns:
            None
        """
        if name_stop:
            self.name = name_stop
        clear_line(parallel=self.parallel)
        print_stats(self.name, status, parallel=self.parallel)
        return True

    def abort(self, name_abort: str = "Aborting", status: str = "[KO]") -> bool:
        """
        A function to abort the program, clearing the line, printing the action to abort, and exiting with status 1.
        Parameters:
            action_abort (str): The action to abort.
        Returns:
            None
        """
        clear_line(parallel=self.parallel)
        print_stats(name_abort, status, parallel=self.parallel)
        return False
