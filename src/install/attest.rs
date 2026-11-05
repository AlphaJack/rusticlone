//! GitHub's build attestations: proof, signed through Sigstore, that this repository's release workflow
//! built an archive. Checked against the production trust root that `sigstore-verify` embeds, so that only
//! the attestation is fetched, and with `ring`, which the rest of rusticlone already uses.

use anyhow::Result;
use ring::{digest, signature};
use serde::Deserialize;
use sigstore_verify::{
    VerificationPolicy,
    crypto::{self, CryptoProvider, HashContext, SigningScheme},
    trust_root::{SIGSTORE_PRODUCTION_TRUSTED_ROOT, TrustedRoot},
    types::{Bundle, HashAlgorithm, Sha256Hash},
};

use super::download::fetch;

/// The workflow that dist writes, which attests each archive it builds.
const WORKFLOW: &str = ".github/workflows/release.yml";
/// Who vouches for the identity of a GitHub workflow.
const ISSUER: &str = "https://token.actions.githubusercontent.com";

#[derive(Debug, thiserror::Error, crate::i18n::Fluent)]
#[fluent(prefix = "attest-error")]
enum Error {
    Missing { archive: String, source: anyhow::Error },
    Unattested { archive: String, workflow: String, error: String },
}

/// The part of GitHub's answer that is read.
#[derive(Deserialize)]
struct Attestations {
    attestations: Vec<Attestation>,
}

#[derive(Deserialize)]
struct Attestation {
    bundle: serde_json::Value,
}

/// Checks that the release workflow of `repository`, run for `tag`, built `data`, downloaded from `archive`.
pub(crate) fn verify(repository: &str, tag: &str, archive: &str, data: &[u8]) -> Result<()> {
    let hex = crate::sys::sha256(data);
    let api = repository.replace("://github.com/", "://api.github.com/repos/");
    let missing = |source| Error::Missing { archive: archive.to_owned(), source };
    let found = fetch(&format!("{api}/attestations/sha256:{hex}")).map_err(missing)?;
    let workflow = format!("{repository}/{WORKFLOW}@refs/tags/{tag}");
    check(&found, &hex, &workflow).map_err(|error| Error::Unattested {
        archive: archive.to_owned(),
        workflow,
        error,
    })?;
    Ok(())
}

/// Whether one of the attestations in GitHub's answer `found` proves that `workflow` built the file of
/// SHA-256 `hex`. GitHub may also hold attestations of other kinds for the same file. Otherwise, why not.
fn check(found: &[u8], hex: &str, workflow: &str) -> Result<(), String> {
    // Already installed by an earlier check in this process.
    _ = crypto::install_default_provider(&Ring);
    let found: Attestations = serde_json::from_slice(found).map_err(|e| e.to_string())?;
    let policy = VerificationPolicy::any_identity().require_identity(workflow).require_issuer(ISSUER);
    let root = TrustedRoot::from_json(SIGSTORE_PRODUCTION_TRUSTED_ROOT).map_err(|e| e.to_string())?;
    let hash = Sha256Hash::from_hex(hex).map_err(|e| e.to_string())?;
    let mut why = String::new();
    for attestation in found.attestations {
        let checked = Bundle::from_json(&attestation.bundle.to_string())
            .map_err(|e| e.to_string())
            .and_then(|bundle| sigstore_verify::verify(hash, &bundle, &policy, &root).map_err(|e| e.to_string()));
        match checked {
            Ok(_) => return Ok(()),
            Err(e) => why = e,
        }
    }
    Err(why)
}

/// Verification only, as attestations need: signing and keys are left to the defaults, which refuse.
#[derive(Debug)]
struct Ring;

struct Hash(digest::Context);

impl HashContext for Hash {
    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    fn finalize(self: Box<Self>) -> Vec<u8> {
        self.0.finish().as_ref().to_vec()
    }
}

impl CryptoProvider for Ring {
    fn hash(&self, algorithm: HashAlgorithm) -> crypto::Result<Box<dyn HashContext>> {
        let algorithm = match algorithm {
            HashAlgorithm::Sha2256 => &digest::SHA256,
            HashAlgorithm::Sha2384 => &digest::SHA384,
            HashAlgorithm::Sha2512 => &digest::SHA512,
            other => return Err(crypto::Error::UnsupportedAlgorithm(format!("{other:?}"))),
        };
        Ok(Box::new(Hash(digest::Context::new(algorithm))))
    }

    #[expect(clippy::wildcard_enum_match_arm, reason = "ring has no other scheme that Sigstore uses")]
    fn verify_raw(&self, scheme: SigningScheme, key: &[u8], data: &[u8], sig: &[u8]) -> crypto::Result<()> {
        let algorithm: &dyn signature::VerificationAlgorithm = match scheme {
            SigningScheme::EcdsaP256Sha256 => &signature::ECDSA_P256_SHA256_ASN1,
            SigningScheme::EcdsaP256Sha384 => &signature::ECDSA_P256_SHA384_ASN1,
            SigningScheme::EcdsaP384Sha256 => &signature::ECDSA_P384_SHA256_ASN1,
            SigningScheme::EcdsaP384Sha384 => &signature::ECDSA_P384_SHA384_ASN1,
            SigningScheme::Ed25519 => &signature::ED25519,
            SigningScheme::RsaPssSha256 => &signature::RSA_PSS_2048_8192_SHA256,
            SigningScheme::RsaPssSha384 => &signature::RSA_PSS_2048_8192_SHA384,
            SigningScheme::RsaPssSha512 => &signature::RSA_PSS_2048_8192_SHA512,
            SigningScheme::RsaPkcs1Sha256 => &signature::RSA_PKCS1_2048_8192_SHA256,
            SigningScheme::RsaPkcs1Sha384 => &signature::RSA_PKCS1_2048_8192_SHA384,
            SigningScheme::RsaPkcs1Sha512 => &signature::RSA_PKCS1_2048_8192_SHA512,
            // ML-DSA, and the schemes to come.
            _ => return Err(crypto::Error::UnsupportedAlgorithm(format!("{scheme:?}"))),
        };
        let verified = signature::UnparsedPublicKey::new(algorithm, key).verify(data, sig);
        verified.map_err(|_unspecified| crypto::Error::Verification(format!("{scheme:?}")))
    }

    /// Only a timestamp authority's signature is checked over a digest, and GitHub's attestations of public
    /// repositories have none: ring cannot.
    fn verify_prehashed(&self, scheme: SigningScheme, _: &[u8], _: &[u8], _: &[u8]) -> crypto::Result<()> {
        Err(crypto::Error::UnsupportedAlgorithm(crate::i18n::t!("attest-over-digest", scheme = format!("{scheme:?}"))))
    }
}

#[cfg(test)]
mod tests {
    use super::check;

    /// Each scheme that ring signs with is checked with ring's algorithm for it: a signature of the data passes, the
    /// same of other data fails.
    #[test]
    fn each_scheme_checks_its_own_signatures() {
        use ring::{
            rand::SystemRandom,
            signature::{self, EcdsaKeyPair, Ed25519KeyPair, KeyPair},
        };
        use sigstore_verify::crypto::{CryptoProvider, SigningScheme as Scheme};
        let (rng, data) = (SystemRandom::new(), b"attested");
        let checks = |scheme: Scheme, key: &[u8], sig: &[u8]| {
            assert!(super::Ring.verify_raw(scheme, key, data, sig).is_ok(), "{scheme:?}");
            assert!(super::Ring.verify_raw(scheme, key, b"other", sig).is_err(), "{scheme:?} of other data");
        };
        for (scheme, algorithm) in [
            (Scheme::EcdsaP256Sha256, &signature::ECDSA_P256_SHA256_ASN1_SIGNING),
            (Scheme::EcdsaP384Sha384, &signature::ECDSA_P384_SHA384_ASN1_SIGNING),
        ] {
            let pair = EcdsaKeyPair::from_pkcs8(
                algorithm,
                EcdsaKeyPair::generate_pkcs8(algorithm, &rng).unwrap().as_ref(),
                &rng,
            )
            .unwrap();
            checks(scheme, pair.public_key().as_ref(), pair.sign(&rng, data).unwrap().as_ref());
        }
        let pair = Ed25519KeyPair::from_pkcs8(Ed25519KeyPair::generate_pkcs8(&rng).unwrap().as_ref()).unwrap();
        checks(Scheme::Ed25519, pair.public_key().as_ref(), pair.sign(data).as_ref());
        // ring cannot generate an RSA key: a throwaway one, made with openssl.
        let pair =
            signature::RsaKeyPair::from_pkcs8(include_bytes!("../../tests/fixtures/attest-rsa-2048.pk8")).unwrap();
        let paddings: [(Scheme, &dyn signature::RsaEncoding); 6] = [
            (Scheme::RsaPssSha256, &signature::RSA_PSS_SHA256),
            (Scheme::RsaPssSha384, &signature::RSA_PSS_SHA384),
            (Scheme::RsaPssSha512, &signature::RSA_PSS_SHA512),
            (Scheme::RsaPkcs1Sha256, &signature::RSA_PKCS1_SHA256),
            (Scheme::RsaPkcs1Sha384, &signature::RSA_PKCS1_SHA384),
            (Scheme::RsaPkcs1Sha512, &signature::RSA_PKCS1_SHA512),
        ];
        for (scheme, padding) in paddings {
            let mut sig = vec![0; pair.public().modulus_len()];
            pair.sign(padding, &rng, data, &mut sig).unwrap();
            checks(scheme, pair.public().as_ref(), &sig);
        }
        // ring cannot sign with a curve and a hash that differ: signed with openssl, as `openssl dgst -sha384 -sign`.
        let bytes = |hex: &str| -> Vec<u8> {
            hex.as_bytes().chunks(2).map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap()).collect()
        };
        for (scheme, key, sig) in [
            (
                Scheme::EcdsaP256Sha384,
                "04a0c3f5f08fc6b21aa8af59ea9312af271b74d9b4d7c9a09835edbf397787aaf7048428543a417f5caf4ad23a609abb18666f0d2515e102b7cab1fdd05dc3358a",
                "304502203029eaee33b7c9cd49b0a6399164567f0327cf9d2fbb16dfc6c6d6ec4a8c4b1b0221008d750e72239d0e65936b28717c4b2d9b78cf6779f1d0a3b8aa0d20925e970cff",
            ),
            (
                Scheme::EcdsaP384Sha256,
                "04b1c8d117100dda6e34c092885ca96b98ae5deb1c17df4987e13661f2d75cea1b170d6d6bbd3bf20d5d9111eb35d77ab3f5a57add5621ed59a0200e096eb50a2fe6401382302a21b4a02af8954a54698ce607c82805f67f7c5d18e79f6f379e1b",
                "306502300c81a720c65e3fbe96596f1e39e2696c6f45530009a3026134c09dc24ffa1dfb66e0bf65d3c67965a5904c71ca92861e023100e34e4b24d12310c4109aae650e5e3d6c2e46247a4b1f135f714ea79d132398f0d5e638aad6ffd5d5adb26e4aab36f3ed",
            ),
        ] {
            checks(scheme, &bytes(key), &bytes(sig));
        }
        assert!(super::Ring.verify_prehashed(Scheme::Ed25519, b"", b"", b"").is_err(), "over a digest");
    }

    /// A release of uv, attested by its own workflow.
    const UV: &[u8] = include_bytes!("../../tests/fixtures/attestations-uv.json");
    const UV_HEX: &str = "7906c79adb21c81abd559841ba76bb1d9af9d791acac7a041b2972e03ae16538";
    const UV_WORKFLOW: &str = "https://github.com/astral-sh/uv/.github/workflows/release.yml@refs/heads/main";

    #[test]
    fn an_attestation_proves_which_workflow_built_which_file() {
        assert_eq!(check(UV, UV_HEX, UV_WORKFLOW), Ok(()));
        let other_workflow = UV_WORKFLOW.replace("astral-sh", "someone-else");
        assert!(check(UV, UV_HEX, &other_workflow).is_err_and(|why| why.contains("someone-else")));
        let other_file = UV_HEX.replace('7', "8");
        assert!(check(UV, &other_file, UV_WORKFLOW).is_err());
    }

    /// GitHub's answer is checked against the repository's own workflow, run for the tag.
    #[test]
    fn an_archive_is_checked_against_the_attestations_found() {
        let (repository, asked) = crate::install::download::serve(200, UV);
        let found = super::verify(&repository, "v1.0.0", "uv.tar.gz", b"uv").unwrap_err().to_string();
        assert!(found.contains(&format!("{repository}/.github/workflows/release.yml@refs/tags/v1.0.0")), "{found}");
        let hex = crate::sys::sha256(b"uv");
        assert_eq!(crate::install::download::asked(&asked), format!("/attestations/sha256:{hex}"));
        let (repository, _) = crate::install::download::serve(404, b"Not Found");
        let missing = super::verify(&repository, "v1.0.0", "uv.tar.gz", b"uv").unwrap_err();
        assert!(missing.downcast_ref::<super::Error>().is_some_and(|e| matches!(e, super::Error::Missing { .. })));
    }

    /// dist's workflow is the one that attests.
    #[test]
    #[expect(clippy::disallowed_methods, reason = "reads the workflow that the constant names")]
    fn the_release_workflow_attests_its_archives() {
        let workflow = std::fs::read_to_string(crate::tests::root().join(super::WORKFLOW));
        assert!(workflow.unwrap().contains("actions/attest"));
    }
}
