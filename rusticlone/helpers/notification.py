"""
Notify user using apprise
"""

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of notification.py                                   │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── CLASSES
# ├── FUNCTIONS
# │
# └───────────────────────────────────────────────────────────────

# ################################################################ IMPORTS

# push notifications
try:
    import apprise
except ImportError:
    HAS_APPRISE = False
else:
    HAS_APPRISE = True

# ################################################################ CLASSES


class Result:
    """
    Result of an atomic operation
    """

    def __init__(
        self, profile: str, operation: str, success: bool, duration: str
    ) -> None:
        """
        Initialize the result
        """
        self.profile = profile
        self.operation = operation
        self.success = success
        self.duration = duration


# ################################################################ FUNCTIONS


def notify_user(results: dict[str, Result], apprise_urls: list[str]) -> None:
    """
    Check if apprise is installed
    """
    if HAS_APPRISE:
        message = create_message(results)
        send_message(message, apprise_urls)
    else:
        print("Apprise is not installed")


def create_message(results: dict[str, Result]) -> str:
    """
    Destructure results to create a message
    """
    message = "profile,operation,success,duration\n"
    for result in results.values():
        message += (
            f"{result.profile},{result.operation},{result.success},{result.duration}\n"
        )
    return message


def send_message(message: str, apprise_urls: list[str]) -> None:
    """
    Send the message to the different services using Apprise
    """
    dispatcher = apprise.Apprise()
    dispatcher.add(url for url in apprise_urls)
    dispatcher.notify(
        title="Rusticlone copleted",
        body=message,
    )
