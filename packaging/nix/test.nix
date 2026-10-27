{ pkgs }:

let
  home-manager = builtins.fetchTarball {
    url = "https://github.com/nix-community/home-manager/archive/7db163b2aee7e410e396b48aed02f646d742b9b8.tar.gz";
    sha256 = "09wipd3n3xlqjcdyqq66wx9vd6ilizcz5v8jdfh2029ymm2m8xbx";
  };
  password = pkgs.writeText "password" "test";
in
{
  nixos = pkgs.testers.runNixOSTest {
    name = "rusticlone";
    nodes.machine = {
      imports = [ ./nixos.nix ];
      environment.systemPackages = [ pkgs.curl ];
      system.activationScripts.secrets = {
        deps = [ "specialfs" ];
        text = ''
          install -d -m 0751 /run/secrets
          install -m 0400 ${pkgs.writeText "documents" "test"} /run/secrets/documents
          install -m 0400 ${pkgs.writeText "Docs-cloud.toml" ''
            [repository]
            repository = "/var/backup/cloud"
            password = "test"
          ''} /run/secrets/Docs-cloud.toml
          install -m 0400 ${pkgs.writeText "rusticlone.env" "RUSTICLONE_ADMIN_TOKEN=secret-token"} /run/secrets/rusticlone.env
        '';
      };
      programs.rustic.profiles = {
        Docs-hdd = {
          backup.snapshots = [ { sources = [ "/data/docs" ]; } ];
          repository = {
            repository = "/var/backup/hdd";
            password-file = "/run/secrets/documents";
          };
          copy.targets = [ "Docs-cloud" ];
          global.env.RUSTICLONE_BACKUP_CRON = "* * * * *";
        };
      };
      programs.rustic.profileFiles.Docs-cloud = "/run/secrets/Docs-cloud.toml";
      services.rusticlone = {
        enable = true;
        environmentFile = "/run/secrets/rusticlone.env";
      };
    };
    testScript = ''
      machine.succeed("mkdir -p /data/docs && echo hello > /data/docs/a.txt")
      machine.wait_for_unit("rusticlone.service")
      machine.wait_until_succeeds("ls /var/backup/hdd/snapshots | grep .", timeout=900)
      machine.succeed("rusticlone backup --all")
      machine.succeed("ls /var/backup/cloud/snapshots | grep .")
      machine.fail("touch /etc/rustic/Other.toml")
      status = "curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:32110/status"
      machine.succeed(f"test $({status}) = 401")
      machine.succeed(f"test $({status} -H 'Authorization: Bearer secret-token') = 200")
      machine.succeed("echo changed > /data/docs/a.txt")
      machine.succeed("rusticlone restore --all")
      machine.succeed("grep hello /data/docs/a.txt")
      machine.succeed("rm -r /data/docs /var/backup/hdd")
      machine.succeed("rusticlone restore --all")
      machine.succeed("grep hello /data/docs/a.txt")
      machine.succeed("echo again > /data/docs/b.txt")
      copied = machine.succeed("ls /var/backup/cloud/snapshots | wc -l")
      machine.succeed("rusticlone backup --all")
      machine.succeed(f"test $(ls /var/backup/cloud/snapshots | wc -l) -gt {copied}")
    '';
  };

  home = pkgs.testers.runNixOSTest {
    name = "rusticlone-home";
    nodes.machine = {
      imports = [ "${home-manager}/nixos" ];
      users.users.jack = {
        isNormalUser = true;
        linger = true;
      };
      home-manager = {
        useGlobalPkgs = true;
        useUserPackages = true;
        users.jack = {
          imports = [ ./home.nix ];
          home.stateVersion = "26.05";
          programs.rustic.profiles = {
            Documents-hdd = {
              backup.snapshots = [ { sources = [ "/home/jack/Documents" ]; } ];
              repository = {
                repository = "/home/jack/backup/hdd";
                password-file = "${password}";
              };
              copy.targets = [ "Documents-cloud" ];
              global.env.RUSTICLONE_BACKUP_CRON = "* * * * *";
            };
            Documents-cloud.repository = {
              repository = "/home/jack/backup/cloud";
              password-file = "${password}";
            };
          };
          services.rusticlone.enable = true;
        };
      };
    };
    testScript = ''
      def jack(command):
          return machine.succeed(f"su - jack -c '{command}'")

      machine.wait_for_unit("multi-user.target")
      jack("mkdir Documents && echo hello > Documents/a.txt")
      machine.wait_for_unit("rusticlone.service", "jack")
      machine.wait_until_succeeds("ls /home/jack/backup/hdd/snapshots | grep .", timeout=900)
      jack("rusticlone backup --all")
      jack("ls backup/cloud/snapshots | grep .")
      jack("echo changed > Documents/a.txt")
      jack("rusticlone restore --all")
      jack("grep hello Documents/a.txt")
    '';
  };
}
