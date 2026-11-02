# Nix

The flake's modules take your rustic profiles as Nix, keep them read-only, and start the server with rustic and rclone:

```nix
# flake.nix
inputs.rusticlone.url = "github:AlphaJack/rusticlone";
```

## NixOS

The NixOS module writes the profiles to `/etc/rustic` and runs the server as root.
On macOS, nix-darwin's `inputs.rusticlone.darwinModules.default` does the same.

```nix
imports = [ inputs.rusticlone.nixosModules.default ];
# profile 1: secret passed via file
programs.rustic.profiles.Documents-hdd = {
  backup.snapshots = [ { sources = [ "/home/jack/Documents" ]; } ];
  repository = {
    repository = "/mnt/hdd/backups/Documents";
    password-file = config.sops.secrets.documents.path;
  };
  copy.targets = [ "Documents-b2" ];
  forget = { keep-last = 10; keep-daily = 7; keep-monthly = 12; };
  global.env = {
    RUSTICLONE_BACKUP_CRON = "@daily";
    RUSTICLONE_CHECK_CRON = "@weekly";
    RUSTICLONE_CHECK_SUBSETS = "4";
    RUSTICLONE_PRUNE_CRON = "@monthly";
  };
};
# profile 2: secrets in toml
programs.rustic.profileFiles.Documents-b2 = config.sops.templates."Documents-b2.toml".path;
services.rusticlone = {
  enable = true;
  # RUSTICLONE_ADMIN_TOKEN and RUSTICLONE_METRICS_TOKEN
  environmentFile = config.sops.secrets.rusticlone.path;
};
```

To [restore](restore.md), run `sudo rusticlone restore` with the flags you need or use the web UI.
After a disaster, reistall NixOS and run `sudo rusticlone restore --all` to bring back the files.

## Home Manager

The Home Manager module writes the profiles to the unprivileged folder and runs the server as your user at login with its tray icon.
It takes the same `programs.rustic` and `services.rusticlone` options:

```nix
imports = [ inputs.rusticlone.homeModules.default ];
```

Restore with `rusticlone restore` without `sudo`.
macOS grants Full Disk Access to an executable by its path, so you need to grant it again to the new `rusticlone` afterwards.
