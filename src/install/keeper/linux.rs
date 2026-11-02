//! Linux's keepers: containers.

use super::Keeper;

#[test]
fn a_container_names_its_tool_and_image() {
    let podman = Keeper::Podman.to_string();
    assert!(podman.contains("podman pull ghcr.io/alphajack/rusticlone:latest"), "{podman}");
    assert!(Keeper::Docker.to_string().contains("docker pull"), "{}", Keeper::Docker);
}
