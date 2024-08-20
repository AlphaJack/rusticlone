# Rusticlone

<p style='text-align: center;'>
<strong>3-2-1 backups using Rustic and RClone</strong>
</p>

<div style='text-align: left;'>
    <img alt="backup process divided in archive and upload" src="https://github.com/AlphaJack/rusticlone/raw/master/images/process-backup.png" style="width: 43%; vertical-align: top;"/>
    <img alt="output of rusticlone backup parallel" src="https://github.com/AlphaJack/rusticlone/raw/master/images/parallel-backup.png" style="width: 40%; vertical-align: top;"/>
    <img alt="output of rusticlone backup sequential" src="https://github.com/AlphaJack/rusticlone/raw/master/images/sequential-backup.png" style="width: 14%; vertical-align: top;"/>
    <br/>
    <img alt="restore process divided in download and extract" src="https://github.com/AlphaJack/rusticlone/raw/master/images/process-restore.png" style="width: 43%; vertical-align: top;"/>
    <img alt="output of rusticlone restore parallel" src="https://github.com/AlphaJack/rusticlone/raw/master/images/parallel-restore.png" style="width: 40%; vertical-align: top;"/>
    <img alt="output of rusticlone restore sequential" src="https://github.com/AlphaJack/rusticlone/raw/master/images/sequential-restore.png" style="width: 14%; vertical-align: top;"/>
</div>

## Motivation

[Rustic](https://rustic.cli.rs/) comes with [native support](https://rustic.cli.rs/docs/commands/init/rclone.html) for [RClone](https://rclone.org/)'s built-in [Restic server](https://rclone.org/commands/rclone_serve_restic/).
After trying this feature, I experienced an abysmally low backup speed, much lower than my upload bandwidth: the bottleneck was the synchronous RClone server, as Rustic was waiting for a response before sending other data.

Another side effect of this feature is that Rustic does not create a local repo, meaning I would have to restore directly from the cloud in case of a disaster.

Since I could not run Rustic once for all my profiles (Documents, Pictures, etc.) I came up with a tool to:

- run Rustic for all my profiles
- archive them to local Rustic repos
- upload local repos to a RClone remote

When restoring, this tool would first download a copy of the RClone remote, and then restore from local Rustic repos.
By decoupling these operations, I got:

- three copies of my data, two of which are local and one is remote (3-2-1 backup strategy)
- the bottlenecks are now the SSD speed (for archive and extract operations) and Internet bandwidth (for upload and download operations)

If it sounds interesting, keep reading!

## Installation

Install [RClone](https://rclone.org/install/) >= 1.67, [Rustic](https://rustic.cli.rs/docs/installation.html) >= 0.7, [Python](https://www.python.org/downloads/) >= 3.10 and then `rusticlone`:

```bash
pip install rusticlone
```

[Configure RClone](https://rclone.org/commands/rclone_config/) by adding a remote.

[Create your Rustic TOML profiles](https://github.com/rustic-rs/rustic/tree/main/config) under "/etc/rustic/" or "$HOME/.config/rustic/" on Linux and MacOS. On Windows, you can put them under "%PROGRAMDATA%/rustic/config" or "%APPDATA%/rustic/config".
Configure your profiles to have a **single source**.
They should also have a local repository destination, without specifying the RClone remote.
You can take inspiration from the profiles in the [example](example/rustic) folder.


Include variables for the location (and password) of the RClone configuration:

```toml
[global.env]
RCLONE_CONFIG="/home/user/.config/rclone/rclone.conf"
RCLONE_CONFIG_PASS="XXXXXX"
```

## Usage
### Backup

Let's assume you want to backup your **PC Documents** to both an **external hard drive** (HDD) and **Google Drive**.

With RClone, you have configured your Google Drive as the <gdrive:/> remote.

You have created the "/etc/rustic/Documents.toml" Rustic profile with:

- source "/home/user/Documents"
- destination "/mnt/backup/Documents" (assuming your external HDD is mounted on "/mnt")

Launch Rusticlone specifying the RClone remote and the `backup` command:

```bash
rusticlone -r "gdrive:/PC" backup
```

Great! You just backed up your documents to both "/mnt/backup/Documents" and <gdrive:/PC/Documents>!

Check the result with the following commands:

```bash
# size of all your documents
du -sh "/home/users/Documents"

# contents of local rustic repo 
rustic -P "Documents" repoinfo
tree "/mnt/backup/Documents"

# contents of remote rustic repo
rclone ncdu "gdrive:/PC/Documents"
```

### Restore
#### From the local Rustic repo

In case you lose your PC, but still have your external HDD, on your new PC you need:

- `rusticlone` and dependencies installed
- your Rustic profiles in place
- your external HDD mounted

Then, run:

```bash
rusticlone extract
```

Great! You just restored your documents from "/mnt/backup/Documents" to "/home/user/Documents".

#### From the RClone remote

In case you lose both your PC files and your external HDD, don't worry! You still have your data on the RClone remote.

On your new PC you need:

- `rusticlone` and dependencies installed
- your RClone configuration
- your Rustic profiles in place
- a new external HDD mounted

Then, run:

```bash
rusticlone -r "gdrive:/PC" restore
```

Fantastic! You downloaded a copy of your Google Drive backup to the external HDD,
and you restored your documents from the HDD to their original location.

Check that everything went well:

```bash
# your remote backup files are still there
rclone ncdu "gdrive:/PC/Documents"

# your new external HDD contains a rustic repo
ls -lah "/mnt/backups/Documents"
rustic -P "Documents" repoinfo

# your documents have been restored
du -sh "/home/users/Documents"
ls -lah "/home/users/Documents"
```

You can now run `rusticlone -r "gdrive:/PC" backup` as always to keep your data safe.

### Individual commands

In alternative to `backup` and `restore`, you can also run individual `rusticlone` commands:

```bash
# use rustic from source to local repo
rusticlone archive

# use rclone from local repo to remote
rusticlone -r "gdrive:/PC" upload

# use rclone from remote to local repo
rusticlone -r "gdrive:/PC" download

# use rustic from local repo to source
rusticlone extract
```

### Parallel processing

You can specify the `--parallel` argument with any command to process all your profiles at the same time:

```bash
rusticlone --parallel -r "gdrive:/PC" backup
```

Beware that this may fill your RAM if you have many profiles or several GB of data to archive.

### Exclude profiles

Rustic has a handy feature: you can create additional profiles to store options shared between profiles.

Let's assume this profile is called "common.toml" and contains the following:

```toml
[forget]
prune = true
keep-last = 1
keep-daily = 7
keep-weekly = 4
keep-monthly = 3
keep-quarter-yearly = 4
keep-yearly = 1
```

This "common.toml" profile can be referenced from our documents by adding to "Documents.toml" the following:

```toml
[global]
use-profile = ["common"]
```

To exclude "common.toml" from Rusticlone (since it cannot be used alone), add the `--ignore` argument followed by "common":

```bash
rusticlone --ignore "common" -r "gdrive:/PC" backup
```

All the profiles containing "common" in their name will be excluded, but will still be sourced from other profiles when needed.

### Custom log file

The default behavior is that, if present, both Rustic and RClone use the log file specified in the Rustic profile configuration.

A custom log file for both Rustic and RClone can be specified with `--log-file`.

```bash
rusticlone --log-file "/var/log/rusticlone.log" archive
```

If no argument is passed and no log file can be found in Rustic configuration, "rusticlone.log" in the current folder is used.

### Automatic system backups

Place your profiles under "/etc/rustic".
If you are storing the RClone password inside the profiles, make sure the folder is only readable by root.

Create a Systemd timer unit "/etc/systemd/system/rusticlone.timer" and copy inside the following:

```ini
[Unit]
Description=Rusticlone timer

[Timer]
# every day at midnight
OnCalendar=*-*-* 00:00:00

[Install]
WantedBy=timers.target
```

Create a Systemd service unit "/etc/systemd/system/rusticlone.service" and copy inside the following:

```ini
[Unit]
Description=Rusticlone service

[Service]
Type=oneshot
ExecStart=rusticlone --ignore "common" --remote "gdrive:/PC" backup
```

Adjust your `--ignore` and `--remote` as needed.

Apply your changes and enable the timer:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now rusticlone.timer
```

## Testing

You can test Rusticlone with dummy files before using it for your precious data:

```bash
bash tests/tests.sh
```

You will need `rclone`, `rustic`, and `python-coverage` installed to run the test.
Before running the test, make sure that you have no important files under "$HOME/.config/rustic".

At the end, you can read a test coverage report with your browser, to see which lines of the source code were run during the test.

## Known limitations

- You must specify **only one source folder per Rustic profile**, as Rustic 0.7.0 does not support array of sources. Wait for Rustic 0.8.0 and Rusticlone 1.2.0 before including arrays. ([feature request](https://github.com/rustic-rs/rustic/issues/1125#issue-2251075638))
- Rustic **does not save file and permissions for the source location**, but only for files and folders **inside the source**. If you backup "/home/jack" with user "jack" and permission "0700", when you will restore it will have user "root" and permission "0755" ([intended behavior](https://github.com/rustic-rs/rustic/issues/1108#issuecomment-2016584568))

## Contribute

Feel free to open an Issue to report bugs or request new features.

Pull Requests are welcome, too!

## License

Licensed under [GPL-3.0](LICENSE) terms.

Not affiliated with Rustic or RClone.

