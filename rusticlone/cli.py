#!/usr/bin/env python

# ┌───────────────────────────────────────────────────────────────┐
# │ Contents of cli.py                                            │
# ├───────────────────────────────────────────────────────────────┘
# │
# ├── IMPORTS
# ├── FUNCTIONS
# ├── ENTRY POINT
# │
# └───────────────────────────────────────────────────────────────

"""
CLI script for Rusticlone
"""

# ################################################################ IMPORTS

# accept arguments
import argparse

# version
# from importlib_metadata import version

# rusticlone
from rusticlone.helpers.custom import load_customizations
from rusticlone.helpers.requirements import check_rustic_version, check_rclone_version

# ################################################################ FUNCTIONS


def parse_args():
    """
    Parse the command-line arguments and return the parsed arguments
    """
    parser = argparse.ArgumentParser(
        prog="rusticlone",
        description="3-2-1 backups using Rustic and RClone",
    )
    # command = parser.add_mutually_exclusive_group(required=True)
    parser.add_argument(
        "command",
        help="backup (archive + upload) or restore (download + extract)",
        nargs=1,
        choices=("archive", "upload", "backup", "download", "extract", "restore"),
    )
    parser.add_argument(
        "-i",
        "--ignore",
        type=str,
        default="🫣🫣🫣",
        help="Ignore rustic profiles containing this pattern",
    )
    parser.add_argument(
        "-l", "--log-file", type=str, help="Log file for Rustic and RClone"
    )
    parser.add_argument(
        "-p", "--parallel", action="store_true", help="Process profiles in parallel"
    )
    group = parser.add_mutually_exclusive_group()
    group.add_argument(
        "-P",
        "--profile",
        nargs=1,
        type=str,
        help="Individual Rustic profile to process",
    )
    parser.add_argument(
        "-r", "--remote", type=str, help="RClone remote and subdirectory"
    )
    # parser.add_argument(
    #    "-v",
    #    "--version",
    #    action="version",
    #    version="%(prog)s " + version("rusticlone"),
    #    help="Show the current version",
    # )
    args = parser.parse_args()
    # https://stackoverflow.com/a/19414853/13448666
    if args.remote is None and args.command[0] in [
        "backup",
        "restore",
        "upload",
        "download",
    ]:
        parser.error(args.command[0] + " requires --remote")
    return args


def main():
    """
    parse arguments, check log file, get a list of profiles and process them
    """
    # parse arguments
    # print(sys.argv)
    args = parse_args()
    if check_rustic_version() and check_rclone_version():
        load_customizations(args)


# ################################################################ ENTRY POINT

if __name__ == "__main__":
    main()
