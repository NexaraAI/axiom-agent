#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalMode {
    Manual,
    Safe,
    Trusted,
}

impl ApprovalMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "manual" => Self::Manual,
            "trusted" => Self::Trusted,
            _ => Self::Safe,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Safe => "safe",
            Self::Trusted => "trusted",
        }
    }
}
