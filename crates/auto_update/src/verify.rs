//! What a downloaded update must prove before it replaces the running app.
//!
//! 1. **Fanta signed it.** The release feed, or a `.sig` file next to the download, carries an
//!    ed25519 signature by Fanta's release key over `fanta-update-v1:{asset}:{version}:{sha256}`.
//!    Binding the asset name and version means an older signed build can't be served as a newer
//!    one, and a build for one platform can't be served to another.
//! 2. **On macOS, Apple agrees.** The new app's code signature verifies strictly and carries the
//!    same Apple Team ID as the running app.
//!
//! A build without a release key (`release-signing-key.pub` has no key line) never checks for
//! updates. `script/update-signing-key` creates the key pair; `script/sign-update` signs a release.

use anyhow::{Context as _, Result, anyhow, ensure};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};
use smol::io::AsyncReadExt as _;
use std::path::Path;
use util::command::new_command;

const SIGNATURE_DOMAIN: &str = "fanta-update-v1";
const RELEASE_KEY_FILE: &str = include_str!("../release-signing-key.pub");

/// Fanta's release-signing public key compiled into this build (raw 32-byte ed25519, base64).
pub(crate) fn compiled_release_key() -> Option<&'static str> {
    key_from_file(RELEASE_KEY_FILE)
}

/// The first line that isn't blank or a `#` comment.
fn key_from_file(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
}

/// The installer the release feed serves for a platform. The backend's release routes and
/// `.github/workflows/release-macos.yml` use the same names.
pub(crate) fn update_asset_name(os: &str, arch: &str) -> String {
    match os {
        "macos" => format!("Fanta-{arch}.dmg"),
        "windows" => format!("Fanta-{arch}.exe"),
        _ => format!("fanta-linux-{arch}.tar.gz"),
    }
}

/// The exact bytes a release signature covers.
pub(crate) fn signed_message(asset: &str, version: &str, sha256_hex: &str) -> String {
    format!("{SIGNATURE_DOMAIN}:{asset}:{version}:{sha256_hex}")
}

pub(crate) async fn sha256_file(path: &Path) -> Result<String> {
    let mut file = smol::fs::File::open(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn verify_signature(
    public_key_b64: &str,
    asset: &str,
    version: &str,
    sha256_hex: &str,
    signature_b64: &str,
) -> Result<()> {
    let engine = base64::engine::general_purpose::STANDARD;
    let key = engine
        .decode(public_key_b64.trim())
        .context("the release key is not base64")?;
    ensure!(
        key.len() == 32,
        "the release key is not an ed25519 public key"
    );
    let signature = engine
        .decode(signature_b64.trim())
        .context("the update signature is not base64")?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
        .verify(
            signed_message(asset, version, sha256_hex).as_bytes(),
            &signature,
        )
        .map_err(|_| anyhow!("the update's signature doesn't match Fanta's release key"))
}

/// Checks a downloaded update: its SHA-256 against the feed's (when the feed gives one), then
/// Fanta's signature over it.
pub(crate) async fn verify_download(
    path: &Path,
    asset: &str,
    version: &str,
    expected_sha256: Option<&str>,
    signature_b64: &str,
    public_key_b64: &str,
) -> Result<()> {
    let actual = sha256_file(path).await?;
    if let Some(expected) = expected_sha256 {
        ensure!(
            expected.trim().eq_ignore_ascii_case(&actual),
            "the download's SHA-256 doesn't match the release feed"
        );
    }
    verify_signature(public_key_b64, asset, version, &actual, signature_b64)
}

/// The Apple Team ID in `codesign --display --verbose=2` output, when a team signed the code.
fn team_identifier(codesign_output: &str) -> Option<&str> {
    codesign_output
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|team| {
            team.len() == 10
                && team
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        })
}

async fn team_of(app: &Path) -> Result<Option<String>> {
    let output = new_command("codesign")
        .args(["--display", "--verbose=2"])
        .arg(app)
        .output()
        .await
        .context("running codesign")?;
    // codesign writes the details to stderr.
    let details = String::from_utf8_lossy(&output.stderr);
    Ok(team_identifier(&details).map(str::to_owned))
}

/// The new app must be validly signed, by an Apple-issued certificate, for the running app's team.
pub(crate) async fn check_macos_code_signature(new_app: &Path, running_app: &Path) -> Result<()> {
    let team = team_of(running_app).await?.context(
        "this copy of Fanta isn't signed by an Apple developer team, so it can't verify updates",
    )?;
    let requirement =
        format!(r#"=anchor apple generic and certificate leaf[subject.OU] = "{team}""#);
    let output = new_command("codesign")
        .args(["--verify", "--deep", "--strict", "-R"])
        .arg(&requirement)
        .arg(new_app)
        .output()
        .await
        .context("running codesign")?;
    ensure!(
        output.status.success(),
        "the update isn't signed by Fanta's Apple team: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use base64::Engine as _;
    use ring::signature::KeyPair as _;

    /// A throwaway release key: (public key, signer) in the formats the updater uses.
    pub(crate) fn release_key() -> (String, impl Fn(&str, &str, &str) -> String) {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let engine = base64::engine::general_purpose::STANDARD;
        let public = engine.encode(pair.public_key().as_ref());
        let sign = move |asset: &str, version: &str, sha256_hex: &str| {
            let message = super::signed_message(asset, version, sha256_hex);
            engine.encode(pair.sign(message.as_bytes()).as_ref())
        };
        (public, sign)
    }

    pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::Digest as _;
        hex::encode(sha2::Sha256::digest(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{release_key, sha256_hex};
    use super::*;

    #[test]
    fn a_signature_covers_the_asset_the_version_and_the_bytes() {
        let (public, sign) = release_key();
        let sha = sha256_hex(b"new build");
        let signature = sign("Fanta-aarch64.dmg", "1.1.0", &sha);
        verify_signature(&public, "Fanta-aarch64.dmg", "1.1.0", &sha, &signature).unwrap();

        let other = sha256_hex(b"tampered build");
        assert!(
            verify_signature(&public, "Fanta-aarch64.dmg", "1.1.0", &other, &signature).is_err()
        );
        assert!(verify_signature(&public, "Fanta-aarch64.dmg", "1.2.0", &sha, &signature).is_err());
        assert!(verify_signature(&public, "Fanta-x86_64.dmg", "1.1.0", &sha, &signature).is_err());

        let (stranger, _) = release_key();
        assert!(
            verify_signature(&stranger, "Fanta-aarch64.dmg", "1.1.0", &sha, &signature).is_err()
        );
        assert!(
            verify_signature(
                "not base64!",
                "Fanta-aarch64.dmg",
                "1.1.0",
                &sha,
                &signature
            )
            .is_err()
        );
        assert!(verify_signature("AAAA", "Fanta-aarch64.dmg", "1.1.0", &sha, &signature).is_err());
        assert!(verify_signature(&public, "Fanta-aarch64.dmg", "1.1.0", &sha, "AAAA").is_err());
    }

    /// Made by `script/update_signing.py sign` with a throwaway key, so the release script and
    /// the updater agree on the key, signature and message formats.
    #[test]
    fn signatures_from_the_release_script_verify() {
        let public = "45Mo2FHJctqmuy+Nl/A0TM0cseSrUzD7oV1joyG8Gmc=";
        let signature = "9pq/fFRIpOBirq8zLO39MozX/Td76mVPW1FrKXpectVpOdC0IEHXAMBMBvkcGpHJQyb9GFlguqznb6r7cmhFAw==";
        let sha = sha256_hex(b"fanta update test vector\n");
        assert_eq!(
            sha,
            "9953fd3da71fa678cbfa283617ffd98d245aa4a0b865383d57cdbdfbd5a40ac5"
        );
        verify_signature(public, "Fanta-aarch64.dmg", "1.1.0", &sha, signature).unwrap();
        assert!(verify_signature(public, "Fanta-aarch64.dmg", "1.1.1", &sha, signature).is_err());
    }

    #[test]
    fn downloads_are_checked_against_the_feed_hash_and_the_signature() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Fanta.dmg");
        std::fs::write(&path, b"new build").unwrap();
        let (public, sign) = release_key();
        let sha = sha256_hex(b"new build");
        let signature = sign("Fanta-aarch64.dmg", "1.1.0", &sha);
        smol::block_on(async {
            verify_download(
                &path,
                "Fanta-aarch64.dmg",
                "1.1.0",
                Some(&sha.to_uppercase()),
                &signature,
                &public,
            )
            .await
            .unwrap();
            verify_download(
                &path,
                "Fanta-aarch64.dmg",
                "1.1.0",
                None,
                &signature,
                &public,
            )
            .await
            .unwrap();
            let wrong = sha256_hex(b"something else");
            let error = verify_download(
                &path,
                "Fanta-aarch64.dmg",
                "1.1.0",
                Some(&wrong),
                &signature,
                &public,
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains("SHA-256"));
        });
    }

    #[test]
    fn the_key_file_ignores_comments_and_blank_lines() {
        assert_eq!(key_from_file("# comment\n\n  # another\n"), None);
        assert_eq!(key_from_file("# Fanta release key\n  abc=\n"), Some("abc="));
    }

    #[test]
    fn asset_names_match_the_release_workflow() {
        assert_eq!(update_asset_name("macos", "aarch64"), "Fanta-aarch64.dmg");
        assert_eq!(
            update_asset_name("linux", "x86_64"),
            "fanta-linux-x86_64.tar.gz"
        );
    }

    #[test]
    fn team_ids_are_read_from_codesign_details() {
        let signed = "Executable=/Applications/Fanta.app/Contents/MacOS/fanta\n\
                      Identifier=dev.fanta.Fanta\nTeamIdentifier=ABCDE12345\nSealed Resources version=2";
        assert_eq!(team_identifier(signed), Some("ABCDE12345"));
        assert_eq!(
            team_identifier("Signature=adhoc\nTeamIdentifier=not set"),
            None
        );
        assert_eq!(team_identifier("TeamIdentifier=abc\"; true"), None);
        assert_eq!(team_identifier("Identifier=x"), None);
    }
}
