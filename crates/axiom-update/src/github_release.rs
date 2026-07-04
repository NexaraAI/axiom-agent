#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCheck {
    pub latest_version: String,
    pub update_available: bool,
}
