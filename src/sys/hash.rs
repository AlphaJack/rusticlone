//! Hashes and random ids.

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

/// Stable across runs, unlike `DefaultHasher`: names lock files and asset versions.
pub(crate) fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(FNV_OFFSET, |h, b| (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME))
}

/// The random bytes of a generated password, token or session id.
#[cfg(feature = "server")]
const RANDOM_BYTES: usize = 32;

/// [`RANDOM_BYTES`] random bytes, hex-encoded.
#[cfg(feature = "server")]
pub(crate) fn random_hex() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; RANDOM_BYTES];
    getrandom::fill(&mut bytes)?;
    Ok(hex(&bytes))
}

#[cfg(any(feature = "server", feature = "self-update"))]
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The SHA-256 of `data`, hex-encoded: sessions are kept by it, and an update checks its download.
#[cfg(any(feature = "server", feature = "self-update"))]
pub(crate) fn sha256(data: impl AsRef<[u8]>) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, data.as_ref()).as_ref())
}
