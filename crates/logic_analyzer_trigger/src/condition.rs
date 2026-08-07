use serde::{Deserialize, Serialize};

/// Portable one-channel condition used by simple logic-analyzer triggers.
///
/// Providers may lower this contract into a native device representation, or evaluate it in a
/// host-side acquisition implementation. Multiple enabled conditions are combined with AND.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SimpleTriggerCondition {
    /// Do not constrain this channel.
    #[default]
    Ignore,
    /// Require the current level to be low.
    Low,
    /// Require the current level to be high.
    High,
    /// Require a low-to-high transition.
    Rising,
    /// Require a high-to-low transition.
    Falling,
    /// Require either level transition.
    Either,
}

impl SimpleTriggerCondition {
    /// Returns whether the condition depends on a prior sample.
    pub const fn is_edge(self) -> bool {
        matches!(self, Self::Rising | Self::Falling | Self::Either)
    }

    /// Tests a previous/current level pair against this condition.
    ///
    /// # Parameters
    /// - `previous`: Prior level, or `None` when no prior sample exists.
    /// - `current`: Current level to evaluate.
    pub const fn matches(self, previous: Option<bool>, current: bool) -> bool {
        match self {
            Self::Ignore => true,
            Self::Low => !current,
            Self::High => current,
            Self::Rising => matches!(previous, Some(false)) && current,
            Self::Falling => matches!(previous, Some(true)) && !current,
            Self::Either => match previous {
                Some(previous) => previous != current,
                None => false,
            },
        }
    }
}

#[cfg(test)]
mod condition_tests {
    use super::SimpleTriggerCondition::{Either, Falling, High, Ignore, Low, Rising};

    #[test]
    fn conditions_match_level_and_edge_samples() {
        assert!(Ignore.matches(None, false));
        assert!(Ignore.matches(Some(true), false));
        assert!(Low.matches(Some(true), false));
        assert!(Low.matches(None, false));
        assert!(!Low.matches(None, true));
        assert!(High.matches(Some(false), true));
        assert!(High.matches(None, true));
        assert!(!High.matches(None, false));
        assert!(Rising.matches(Some(false), true));
        assert!(!Rising.matches(None, true));
        assert!(!Rising.matches(Some(true), true));
        assert!(Falling.matches(Some(true), false));
        assert!(!Falling.matches(None, false));
        assert!(Either.matches(Some(false), true));
        assert!(Either.matches(Some(true), false));
        assert!(!Either.matches(Some(true), true));
        assert!(!Either.matches(None, true));
    }
}
