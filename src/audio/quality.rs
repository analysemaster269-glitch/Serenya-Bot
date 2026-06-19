use std::str::FromStr;

use crate::utils::SerenyaError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(dead_code)]
pub enum Quality {
    Performance,
    #[default]
    Balanced,
    High,
}

#[allow(dead_code)]
impl Quality {
    pub fn display_name(self) -> &'static str {
        match self {
            Quality::Performance => "Performance (64kbps)",
            Quality::Balanced => "Balanced (128kbps)",
            Quality::High => "High Quality (256kbps)",
        }
    }

    pub fn to_bitrate(self) -> u32 {
        match self {
            Quality::Performance => 64000,
            Quality::Balanced => 128000,
            Quality::High => 256000,
        }
    }
}

impl FromStr for Quality {
    type Err = SerenyaError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "performance" | "perf" | "low" => Ok(Quality::Performance),
            "balanced" | "default" | "mid" => Ok(Quality::Balanced),
            "quality" | "high" | "best" => Ok(Quality::High),
            _ => Err(SerenyaError::Config(format!(
                "Invalid quality mode: '{}'. Use 'performance', 'balanced', or 'quality'.",
                s
            ))),
        }
    }
}
