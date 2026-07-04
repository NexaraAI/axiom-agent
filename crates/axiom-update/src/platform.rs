use crate::UpdateError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformAsset {
    pub os: String,
    pub arch: String,
    pub asset_name: String,
}

pub fn current_platform_asset() -> Result<PlatformAsset, UpdateError> {
    resolve_platform_asset(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn resolve_platform_asset(os: &str, arch: &str) -> Result<PlatformAsset, UpdateError> {
    let asset_name = match (os, arch) {
        ("windows", "x86_64" | "x64") => "axiom-x86_64-pc-windows-msvc.exe",
        ("linux", "x86_64" | "x64") => "axiom-x86_64-unknown-linux-gnu",
        ("macos", "x86_64" | "x64") => "axiom-x86_64-apple-darwin",
        ("macos", "aarch64" | "arm64") => "axiom-aarch64-apple-darwin",
        _ => {
            return Err(UpdateError::UnsupportedPlatform {
                os: os.to_string(),
                arch: arch.to_string(),
            })
        }
    };

    Ok(PlatformAsset {
        os: os.to_string(),
        arch: arch.to_string(),
        asset_name: asset_name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_supported_platform_assets() {
        assert_eq!(
            resolve_platform_asset("windows", "x86_64")
                .expect("windows")
                .asset_name,
            "axiom-x86_64-pc-windows-msvc.exe"
        );
        assert_eq!(
            resolve_platform_asset("linux", "x86_64")
                .expect("linux")
                .asset_name,
            "axiom-x86_64-unknown-linux-gnu"
        );
        assert_eq!(
            resolve_platform_asset("macos", "x86_64")
                .expect("macos x64")
                .asset_name,
            "axiom-x86_64-apple-darwin"
        );
        assert_eq!(
            resolve_platform_asset("macos", "aarch64")
                .expect("macos arm")
                .asset_name,
            "axiom-aarch64-apple-darwin"
        );
    }

    #[test]
    fn unsupported_platform_returns_clean_error() {
        assert!(matches!(
            resolve_platform_asset("freebsd", "x86_64"),
            Err(UpdateError::UnsupportedPlatform { .. })
        ));
    }
}
