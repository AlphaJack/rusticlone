{ config, lib, ... }:

let
  rustic = config.programs.rustic;
  cfg = config.services.rusticlone;
in
{
  imports = [ ./common.nix ];

  config = lib.mkMerge [
    (lib.mkIf rustic.enable {
      environment.etc.rustic.source = rustic.dir;
      environment.systemPackages = [ rustic.package ];
    })
    (lib.mkIf cfg.enable {
      programs.rustic.enable = true;
      assertions = [
        {
          assertion = cfg.environmentFile == null;
          message = "launchd reads no environment file: set services.rusticlone.settings.RUSTICLONE_ADMIN_TOKEN_FILE to the file of the token";
        }
      ];
      environment.systemPackages = [ cfg.package ];
      launchd.daemons.rusticlone.serviceConfig = {
        ProgramArguments = [
          (lib.getExe cfg.package)
          "serve"
        ];
        EnvironmentVariables = cfg.settings;
        WorkingDirectory = "/etc/rustic";
        KeepAlive.SuccessfulExit = false;
        Nice = 10;
        LowPriorityIO = true;
      };
    })
  ];
}
