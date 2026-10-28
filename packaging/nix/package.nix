{
  lib,
  stdenv,
  rustPlatform,
  makeBinaryWrapper,
  installShellFiles,
  rustic,
  rclone,
  tzdata,
}:

let
  cargo = lib.importTOML ../../Cargo.toml;
  name = cargo.package.name;
  tested = lib.versions.majorMinor (lib.removePrefix "v" cargo.workspace.metadata.rustic.version);
in
assert lib.assertMsg (lib.versionAtLeast rustic.version tested)
  "rustic ${rustic.version} is older than ${tested}, which Cargo.toml pins: nix flake update";
rustPlatform.buildRustPackage {
  pname = name;
  inherit (cargo.workspace.package) version;

  src = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.difference ../.. (
      lib.fileset.unions [
        ../../screenshots
        ../../packaging/nix
        ../../flake.nix
        ../../flake.lock
        ../../.github
      ]
    );
  };
  cargoLock.lockFile = ../../Cargo.lock;

  buildNoDefaultFeatures = true;
  buildFeatures = [ "desktop" ];

  nativeBuildInputs = [
    makeBinaryWrapper
    installShellFiles
  ];
  useNextest = true;
  cargoTestFlags = [ "--lib" ];
  preCheck = ''
    export TZDIR=${tzdata}/share/zoneinfo
  '';

  postInstall = ''
    wrapProgram $out/bin/${name} --prefix PATH : ${
      lib.makeBinPath [
        rustic
        rclone
      ]
    }
  ''
  + lib.optionalString stdenv.hostPlatform.isLinux ''
    for dir in system user; do
      install -Dm644 packaging/linux/${name}.service $out/lib/systemd/$dir/${name}.service
      substituteInPlace $out/lib/systemd/$dir/${name}.service --replace-fail "ExecStart=%N" "ExecStart=$out/bin/${name}"
    done
    install -Dm644 packaging/linux/${name}.desktop -t $out/share/applications
    install -Dm644 assets/images/logo.svg $out/share/icons/hicolor/scalable/apps/${name}.svg
  ''
  + lib.optionalString (stdenv.buildPlatform.canExecute stdenv.hostPlatform) ''
    installShellCompletion --cmd ${name} \
      --bash <(COMPLETE=bash $out/bin/${name}) \
      --zsh <(COMPLETE=zsh $out/bin/${name}) \
      --fish <(COMPLETE=fish $out/bin/${name})
  '';

  meta = {
    inherit (cargo.package) description;
    homepage = cargo.workspace.package.repository;
    license = lib.getLicenseFromSpdxId cargo.workspace.package.license;
    mainProgram = name;
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
