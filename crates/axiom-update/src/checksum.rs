use sha2::{Digest, Sha256};
use std::collections::HashSet;

use crate::UpdateError;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

pub fn expected_sha256_for_asset(checksums: &str, asset_name: &str) -> Result<String, UpdateError> {
    validate_manifest_asset_name(asset_name).map_err(|reason| {
        UpdateError::InvalidChecksumManifest(format!(
            "requested asset name `{asset_name}` is invalid: {reason}"
        ))
    })?;

    let mut entries = HashSet::new();
    let mut expected = None;
    for (index, line) in checksums.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line_number = index + 1;
        let (checksum, name) = parse_checksum_line(line).map_err(|reason| {
            UpdateError::InvalidChecksumManifest(format!("line {line_number}: {reason}"))
        })?;
        if !entries.insert(name) {
            return Err(UpdateError::DuplicateChecksumEntry(name.to_string()));
        }
        if name == asset_name {
            expected = Some(checksum.to_ascii_lowercase());
        }
    }

    expected.ok_or_else(|| UpdateError::MissingChecksum(asset_name.to_string()))
}

fn parse_checksum_line(line: &str) -> Result<(&str, &str), &'static str> {
    if line.len() < 67 || !line.is_ascii() {
        return Err("entry must be ASCII SHA256SUMS format");
    }

    let bytes = line.as_bytes();
    let checksum = &line[..64];
    if !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("checksum must contain exactly 64 hexadecimal characters");
    }
    if bytes[64] != b' ' || !matches!(bytes[65], b' ' | b'*') {
        return Err("checksum and asset must use two spaces or the GNU binary marker");
    }

    let name = &line[66..];
    validate_manifest_asset_name(name)?;
    Ok((checksum, name))
}

fn validate_manifest_asset_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("asset name is empty");
    }
    if matches!(name, "." | "..") {
        return Err("asset name is path-like");
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
    {
        return Err("asset name contains unsupported characters");
    }
    Ok(())
}

pub fn verify_sha256(
    asset_name: &str,
    bytes: &[u8],
    expected: &str,
) -> Result<String, UpdateError> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(UpdateError::InvalidChecksumManifest(
            "expected checksum must contain exactly 64 hexadecimal characters".to_string(),
        ));
    }
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected) {
        Ok(actual)
    } else {
        Err(UpdateError::ChecksumMismatch {
            asset: asset_name.to_string(),
            expected: expected.to_string(),
            actual,
        })
    }
}

pub fn verify_asset_from_sums(
    asset_name: &str,
    bytes: &[u8],
    checksums: &str,
) -> Result<String, UpdateError> {
    let expected = expected_sha256_for_asset(checksums, asset_name)?;
    verify_sha256(asset_name, bytes, &expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_success_and_mismatch() {
        let digest = sha256_hex(b"hello");
        assert_eq!(
            verify_sha256("axiom", b"hello", &digest).expect("ok"),
            digest
        );
        assert!(matches!(
            verify_sha256("axiom", b"hello", &"0".repeat(64)),
            Err(UpdateError::ChecksumMismatch { .. })
        ));
        assert!(matches!(
            verify_sha256("axiom", b"hello", "not-a-sha256"),
            Err(UpdateError::InvalidChecksumManifest(_))
        ));
    }

    #[test]
    fn missing_checksum_blocks_install() {
        assert!(matches!(
            expected_sha256_for_asset(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other\n",
                "axiom"
            ),
            Err(UpdateError::MissingChecksum(_))
        ));
    }

    #[test]
    fn parses_sha256sums_asset_line() {
        let checksum = "a".repeat(64);
        let sums = format!("{checksum}  axiom-x86_64-unknown-linux-gnu\n");

        assert_eq!(
            expected_sha256_for_asset(&sums, "axiom-x86_64-unknown-linux-gnu").expect("checksum"),
            checksum
        );
    }

    #[test]
    fn rejects_malformed_checksum_entries_even_when_target_is_present() {
        let checksum = "a".repeat(64);
        let sums = format!("{checksum}  axiom\nnot-a-checksum  other\n");

        assert!(matches!(
            expected_sha256_for_asset(&sums, "axiom"),
            Err(UpdateError::InvalidChecksumManifest(_))
        ));
    }

    #[test]
    fn rejects_duplicate_checksum_entries() {
        let checksum = "a".repeat(64);
        let sums = format!("{checksum}  axiom\n{checksum} *axiom\n");

        assert!(matches!(
            expected_sha256_for_asset(&sums, "axiom"),
            Err(UpdateError::DuplicateChecksumEntry(name)) if name == "axiom"
        ));
    }

    #[test]
    fn rejects_invalid_or_inexact_asset_names() {
        let checksum = "a".repeat(64);
        for name in [
            "../axiom",
            "nested/axiom",
            " axiom",
            "axiom ",
            "axiom.exe#old",
        ] {
            let sums = format!("{checksum}  {name}\n");
            assert!(matches!(
                expected_sha256_for_asset(&sums, "axiom"),
                Err(UpdateError::InvalidChecksumManifest(_))
            ));
        }

        let sums = format!("{checksum}  axiom-old\n");
        assert!(matches!(
            expected_sha256_for_asset(&sums, "axiom"),
            Err(UpdateError::MissingChecksum(_))
        ));
    }

    #[test]
    fn accepts_uppercase_hash_and_gnu_binary_marker() {
        let checksum = "A".repeat(64);
        let sums = format!("{checksum} *axiom.exe\n");

        assert_eq!(
            expected_sha256_for_asset(&sums, "axiom.exe").expect("checksum"),
            checksum.to_ascii_lowercase()
        );
    }
}
