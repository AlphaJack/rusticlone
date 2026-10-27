{ config, lib, ... }:

let
  rustic = config.programs.rustic;
  cfg = config.services.rusticlone;
  key = "#RUSTICLONE_LISTEN=";
  listen =
    cfg.settings.RUSTICLONE_LISTEN or (lib.removePrefix key (
      lib.findFirst (lib.hasPrefix key) "" (lib.splitString "\n" (builtins.readFile ../rusticlone.env))
    ));
  port = lib.toInt (lib.last (lib.splitString ":" listen));
  host = lib.removeSuffix ":${toString port}" listen;
  loopback = lib.hasPrefix "127." host || host == "[::1]" || host == "localhost";
  any = host == "0.0.0.0" || host == "[::]";
in
{
  imports = [ ./common.nix ];

  options.services.rusticlone = {
    openFirewall = lib.mkEnableOption "the web UI's port in the firewall";
    nginx = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "The virtual host of nginx that serves the web UI, whose TLS and authentication are set in `services.nginx.virtualHosts`.";
      example = "backups.example.org";
    };
  };

  config = lib.mkMerge [
    (lib.mkIf rustic.enable {
      environment.etc.rustic.source = rustic.dir;
      environment.systemPackages = [ rustic.package ];
    })
    (lib.mkIf cfg.enable {
      programs.rustic.enable = true;
      assertions = [
        {
          assertion = !cfg.openFirewall || !loopback;
          message = "services.rusticlone.openFirewall opens port ${toString port}, which the server listens on at ${host} only: set services.rusticlone.settings.RUSTICLONE_LISTEN";
        }
      ];
      environment.systemPackages = [ cfg.package ];
      systemd.packages = [ cfg.package ];
      systemd.services.rusticlone = {
        wantedBy = [ "multi-user.target" ];
        reloadTriggers = [ rustic.dir ];
        environment = cfg.settings;
        serviceConfig.EnvironmentFile = lib.mkIf (cfg.environmentFile != null) cfg.environmentFile;
      };
      networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ port ];
      services.nginx = lib.mkIf (cfg.nginx != null) {
        enable = true;
        virtualHosts.${cfg.nginx}.locations."/" = {
          proxyPass = "http://${if any then "127.0.0.1" else host}:${toString port}";
          recommendedProxySettings = true;
        };
      };
    })
  ];
}
