use sha2::{Digest, Sha256};

use crate::UpdateError;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

pub fn expected_sha256_for_asset(checksums: &str, asset_name: &str) -> Result<String, UpdateError> {
    for line in checksums.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(checksum) = parts.next() else {
            continue;
        };
        let Some(name) = parts.next() else {
            continue;
        };
        let normalized_name = name.trim_start_matches('*');
        if normalized_name == asset_name {
            return Ok(checksum.to_ascii_lowercase());
        }
    }

    Err(UpdateError::MissingChecksum(asset_name.to_string()))
}

pub fn verify_sha256(
    asset_name: &str,
    bytes: &[u8],
    expected: &str,
) -> Result<String, UpdateError> {
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
    }

    #[test]
    fn missing_checksum_blocks_install() {
        assert!(matches!(
            expected_sha256_for_asset("abcd  other\n", "axiom"),
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
}
