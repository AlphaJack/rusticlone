# How it works

`rusticlone --help` lists all commands and possible options, also listed in [Commands](reference/commands.md).

Under the hood, `rusticlone` launches rustic subcommands directly. From the web app, you can see which rustic command and arguments were used for each operation.

Each step that `rusticlone` performs needs one or more resources: CPU, disk, upload or download bandwidth, or exclusive repository access.
The internal scheduler ensures that each resource is used by one step at a time and released as soon as a step finishes.

Commands, the server and its schedule can run at the same time: each run waits for the disk, network link or CPU that another holds.

<img class="light-only" alt="CLI backups at the same time" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/cli-light-holds.png">
<img class="dark-only" alt="CLI backups at the same time" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/cli-dark-holds.png" loading="lazy">

Rusticlone state is meant to be disposable, as the source of truth for your backups remains your repositories. See [Folders](reference/folders.md#state) for where it lives.

Rusticlone can be configured through environment variables ([reference list](reference/settings.md)), set in its [configuration file](reference/folders.md#configuration-file).
They are read at start: restart Rusticlone after editing its configuration file.

Settings for individual collections or repositories are variables in the `[global.env]` section of rustic profiles.
They can be shared across profiles by using the [`[global] use-profiles`](https://rustic.cli.rs/docs/commands/init/configuration_file.html#:~:text=use%2Dprofiles) feature.
Rusticlone looks for profiles in the [same folders as rustic](reference/folders.md#profiles), and in the working directory.
