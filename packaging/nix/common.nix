{
  config,
  lib,
  pkgs,
  ...
}:

let
  rustic = config.programs.rustic;
  toml = pkgs.formats.toml { };
  profiles = pkgs.linkFarm "rustic-profiles" (
    lib.mapAttrsToList (name: profile: {
      name = "${name}.toml";
      path = toml.generate "${name}.toml" profile;
    }) rustic.profiles
    ++ lib.mapAttrsToList (name: path: {
      name = "${name}.toml";
      inherit path;
    }) rustic.profileFiles
  );
in
{
  options.programs.rustic = {
    enable = lib.mkEnableOption "rustic, with its profiles";
    package = lib.mkPackageOption pkgs "rustic" { };
    profiles = lib.mkOption {
      type = lib.types.attrsOf toml.type;
      default = { };
      description = ''
        Rustic's profiles, each the file named after it in rustic's read-only profile directory. The Nix store is
        world-readable: passwords go in `password-file` or `password-command`.
      '';
      example = lib.literalExpression ''
        {
          Documents-hdd = {
            backup.snapshots = [ { sources = [ "/home/jack/Documents" ]; } ];
            repository = {
              repository = "/mnt/hdd/backups/Documents";
              password-file = config.sops.secrets.documents.path;
            };
            copy.targets = [ "Documents-b2" ];
            global.env.RUSTICLONE_BACKUP_CRON = "@daily";
          };
        }
      '';
    };
    profileFiles = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      description = ''
        Profiles kept outside the Nix store, such as agenix secrets or sops-nix templates, each linked from the
        file named after it in rustic's profile directory. For profiles that hold secrets other than the password, like the
        keys of an S3 bucket.
      '';
      example = lib.literalExpression ''{ Documents-b2 = config.sops.templates."Documents-b2.toml".path; }'';
    };
    dir = lib.mkOption {
      type = lib.types.package;
      default = profiles;
      readOnly = true;
      internal = true;
    };
  };

  options.services.rusticlone = {
    enable = lib.mkEnableOption "Rusticlone's server, which runs the collections of `programs.rustic.profiles` on their schedules";
    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.callPackage ./package.nix { rustic = rustic.package; };
      defaultText = lib.literalExpression "Rusticlone, with `programs.rustic.package`";
      description = "The Rusticlone package to run.";
    };
    settings = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      description = "Rusticlone's settings from the environment, which are world-readable in the Nix store.";
      example = {
        RUSTICLONE_LISTEN = "0.0.0.0:32110";
      };
    };
    environmentFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = "A file of settings outside the Nix store, for secrets such as RUSTICLONE_ADMIN_TOKEN.";
      example = "/run/secrets/rusticlone.env";
    };
  };

  config = lib.mkIf rustic.enable {
    assertions =
      lib.mapAttrsToList (name: profile: {
        assertion = !(profile.repository or { }) ? password;
        message = "programs.rustic.profiles.${name}.repository.password would be world-readable in the Nix store: use password-file, password-command or programs.rustic.profileFiles";
      }) rustic.profiles
      ++ lib.mapAttrsToList (name: _: {
        assertion = !rustic.profiles ? ${name};
        message = "programs.rustic.profiles.${name} and programs.rustic.profileFiles.${name} are the same file";
      }) rustic.profileFiles;
  };
}
