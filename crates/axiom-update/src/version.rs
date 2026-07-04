use semver::Version;

pub fn is_newer_version(current: &Version, candidate: &Version) -> bool {
    candidate > current
}
