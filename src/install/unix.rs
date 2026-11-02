//! Unix's keepers: the system, Nix and Homebrew.

#![expect(clippy::disallowed_methods, reason = "lays out package managers' folders")]

use std::path::Path;

use super::{Keeper, keeper, managed};

/// Root can write them, but a package put the executable there.
#[test]
fn the_system_packages_folders_are_managed_for_root_too() {
    for dir in ["/usr/bin", "/bin", "/usr/sbin"] {
        assert_eq!(keeper(&Path::new(dir).join(crate::NAME)), Some(Keeper::System), "{dir}");
    }
}

#[test]
fn nixs_store_is_managed() {
    assert_eq!(
        keeper(&Path::new("/nix").join("store").join("abc-rusticlone-2.0.1").join("bin").join("rusticlone")),
        Some(Keeper::Nix)
    );
}

/// Started through Homebrew's `bin`, macOS names the executable by the link.
#[test]
fn homebrews_cellar_is_managed_through_its_link() {
    let root = std::env::temp_dir().join(format!("{}-brew-{}", crate::NAME, std::process::id()));
    let (cellar, bin) = (root.join("Cellar").join("rusticlone").join("2.0.1").join("bin"), root.join("bin"));
    std::fs::create_dir_all(&cellar).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(cellar.join(crate::NAME), b"").unwrap();
    std::os::unix::fs::symlink(cellar.join(crate::NAME), bin.join(crate::NAME)).unwrap();
    let found = managed(&bin.join(crate::NAME));
    std::fs::remove_dir_all(&root).unwrap();
    assert!(found);
}

#[test]
fn homebrews_cellar_is_managed() {
    for prefix in ["/opt/homebrew", "/usr/local", "/home/linuxbrew/.linuxbrew"] {
        let exe = Path::new(prefix).join("Cellar").join("rusticlone").join("2.0.1").join("bin").join("rusticlone");
        assert_eq!(keeper(&exe), Some(Keeper::Homebrew), "{prefix}");
    }
}
