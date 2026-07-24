use ratatui::style::Color;

use crate::utils::now;

pub const CYAN: Color = Color::Rgb(69, 211, 255);
pub const GREEN: Color = Color::Rgb(116, 235, 152);
pub const YELLOW: Color = Color::Rgb(255, 205, 92);
pub const RED: Color = Color::Rgb(255, 105, 105);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Local,
    Free,
    Paid,
    Cloud,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum CostStatus {
    ProviderReported,
    Calculated,
    Estimated,
    Free,
    Local,
    #[default]
    Unavailable,
}

impl CostStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::ProviderReported => "reported",
            Self::Calculated => "calculated",
            Self::Estimated => "estimated",
            Self::Free => "free",
            Self::Local => "local",
            Self::Unavailable => "unavailable",
        }
    }

    pub fn is_billable(self) -> bool {
        matches!(
            self,
            Self::ProviderReported | Self::Calculated | Self::Estimated
        )
    }

    pub fn is_known(self) -> bool {
        self != Self::Unavailable
    }
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "LOCAL",
            Self::Free => "FREE",
            Self::Paid => "PAID",
            Self::Cloud => "CLOUD",
            Self::Unknown => "UNKNOWN",
        }
    }
    pub fn color(self) -> Color {
        match self {
            Self::Local => GREEN,
            Self::Free => CYAN,
            Self::Paid => YELLOW,
            Self::Cloud => Color::Rgb(194, 137, 255),
            Self::Unknown => RED,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub provider: String,
    pub model: String,
    pub category: Category,
    pub requests: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: Option<f64>,
    pub cost_status: CostStatus,
    pub created: i64,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.input + self.output + self.reasoning + self.cache_read + self.cache_write
    }
}

#[derive(Default)]
pub struct Totals {
    pub requests: u64,
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost: f64,
    pub unknown_requests: u64,
}

impl Totals {
    pub fn add(&mut self, usage: &Usage) {
        self.requests += usage.requests;
        self.input += usage.input;
        self.output += usage.output;
        self.reasoning += usage.reasoning;
        self.cache_read += usage.cache_read;
        self.cache_write += usage.cache_write;
        if !usage.cost_status.is_known() {
            self.unknown_requests += usage.requests;
        }
        if usage.cost_status.is_billable() {
            if let Some(cost) = usage.cost {
                self.cost += cost;
            }
        }
    }
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.reasoning + self.cache_read + self.cache_write
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Range {
    Today,
    Week,
    Month,
    Days(u64),
    All,
}

impl Range {
    pub fn label(self) -> String {
        match self {
            Self::Today => "TODAY".to_string(),
            Self::Week => "7 DAYS".to_string(),
            Self::Month => "30 DAYS".to_string(),
            Self::Days(days) => format!("{} DAYS", days),
            Self::All => "ALL TIME".to_string(),
        }
    }
    pub fn cutoff(self) -> i64 {
        if self == Self::All {
            return 0;
        }
        let seconds: i64 = match self {
            Self::Today => 86_400,
            Self::Week => 604_800,
            Self::Month => 2_592_000,
            Self::Days(days) => days.saturating_mul(86_400).min(i64::MAX as u64) as i64,
            Self::All => 0,
        };
        now().saturating_sub(seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_include_all_token_buckets() {
        let u = Usage {
            input: 2,
            output: 3,
            reasoning: 4,
            cache_read: 5,
            cache_write: 6,
            ..Default::default()
        };
        assert_eq!(u.total_tokens(), 20);
    }

    #[test]
    fn extreme_day_ranges_are_safe() {
        assert!(Range::Days(u64::MAX).cutoff() <= now());
    }
}
