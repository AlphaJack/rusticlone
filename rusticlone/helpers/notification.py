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
    from apprise import Apprise
except (ImportError, ModuleNotFoundError):
    HAS_APPRISE = False
else:
    HAS_APPRISE = True

# rusticlone
from rusticlone.helpers.formatting import print_stats

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


def notify_user(results: dict[str, Result], apprise_url: str) -> None:
    """
    Check if apprise is installed
    """
    if HAS_APPRISE:
        notification = create_notification(results)
        send_notification(notification, apprise_url)
    else:
        print("Please install apprise to send notifications")


def create_notification(results: dict[str, Result]) -> str:
    """
    Destructure results to create a single notification
    """
    lines = []
    for result in results.values():
        status = "✅" if result.success else "🟥"
        status = "🟨" if result.duration == "skipped" else status
        lines.append(f"{status} {result.operation} {result.profile} ({result.duration})")
    notification = "\n".join(lines)
    return notification


def send_notification(notification: str, apprise_url: str) -> None:
    """
    Send the notification to the notification services using Apprise
    """
    dispatcher = Apprise()
    if not dispatcher.add(apprise_url):
        print(f"Invalid Apprise URL: {apprise_url}")
    else:
        service = apprise_url.split('://')[0]
        if not dispatcher.notify(
            title="Rusticlone results:",
            body=notification,
        ):
            print_stats("Notification not sent", f"{service}")
            print(f"\n{notification}")
        else:
            print_stats("Notification sent", f"{service}")
