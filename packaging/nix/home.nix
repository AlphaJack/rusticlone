{
  config,
  lib,
  pkgs,
  ...
}:

let
  rustic = config.programs.rustic;
  cfg = config.services.rusticlone;
  darwin = pkgs.stdenv.hostPlatform.isDarwin;
  exe = lib.getExe cfg.package;
  unit =
    (lib.foldl'
      (
        acc: line:
        let
          section = builtins.match "\\[(.+)]" line;
          pair = builtins.match "([^=]+)=(.*)" line;
        in
        if section != null then
          acc // { section = lib.head section; }
        else if pair != null && !lib.hasPrefix "#" line then
          lib.recursiveUpdate acc { units.${acc.section}.${lib.head pair} = lib.last pair; }
        else
          acc
      )
      {
        section = null;
        units = { };
      }
      (lib.splitString "\n" (builtins.readFile ../linux/rusticlone.service))
    ).units;
in
{
  imports = [ ./common.nix ];

  config = lib.mkMerge [
    (lib.mkIf rustic.enable {
      home.packages = [ rustic.package ];
      xdg.configFile.rustic = lib.mkIf (!darwin) { source = rustic.dir; };
      home.file."Library/Application Support/rustic" = lib.mkIf darwin { source = rustic.dir; };
    })
    (lib.mkIf cfg.enable {
      programs.rustic.enable = true;
      assertions = [
        {
          assertion = !darwin || cfg.environmentFile == null;
          message = "launchd reads no environment file: set services.rusticlone.settings.RUSTICLONE_ADMIN_TOKEN_FILE to the file of the token";
        }
      ];
      home.packages = [ cfg.package ];
      systemd.user.services.rusticlone = lib.mkIf (!darwin) {
        Unit = unit.Unit // {
          X-Reload-Triggers = [ "${rustic.dir}" ];
        };
        Service =
          unit.Service
          // {
            ExecStart = lib.replaceStrings [ "%N" ] [ exe ] unit.Service.ExecStart;
            Environment = lib.mapAttrsToList (name: value: "${name}=${value}") cfg.settings;
          }
          // lib.optionalAttrs (cfg.environmentFile != null) { EnvironmentFile = cfg.environmentFile; };
        Install.WantedBy = [ "default.target" ];
      };
      launchd.agents.rusticlone = lib.mkIf darwin {
        enable = true;
        config = {
          ProgramArguments = [
            exe
            "serve"
          ];
          EnvironmentVariables = cfg.settings;
          WorkingDirectory = "${config.home.homeDirectory}/Library/Application Support/rustic";
          KeepAlive.SuccessfulExit = false;
          ProcessType = "Background";
        };
      };
    })
  ];
}
