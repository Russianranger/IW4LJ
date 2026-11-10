//! How good a bot is. One table, read by perception, the controller and the
//! motor, so a difficulty is a single choice rather than a set of constants
//! spread over three files.

/// The host's difficulty choice. `bot skill <level>` sets it for every bot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkillLevel {
    Recruit,
    #[default]
    Regular,
    Hardened,
    Veteran,
}

impl SkillLevel {
    pub const ALL: [Self; 4] = [Self::Recruit, Self::Regular, Self::Hardened, Self::Veteran];

    /// Accepts the level's name, the retail difficulty names and `0`..`3`.
    pub fn from_token(token: &str) -> Option<Self> {
        Some(match token.trim().to_ascii_lowercase().as_str() {
            "0" | "recruit" | "easy" => Self::Recruit,
            "1" | "regular" | "normal" | "medium" => Self::Regular,
            "2" | "hardened" | "hard" => Self::Hardened,
            "3" | "veteran" | "expert" => Self::Veteran,
            _ => return None,
        })
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Recruit => "recruit",
            Self::Regular => "regular",
            Self::Hardened => "hardened",
            Self::Veteran => "veteran",
        }
    }
}

/// One bot's numbers. Built from the host's level and nudged per bot, so a
/// lobby of the same difficulty is not a lobby of clones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BotSkill {
    pub level: SkillLevel,
    /// Authority ticks (50 ms) between first seeing an enemy and the first shot.
    pub reaction_ticks: u32,
    /// Half-angle, in degrees, the settled aim wanders inside.
    pub aim_error_deg: f32,
    /// Extra error the moment a target is acquired; it decays over `settle_ms`.
    pub acquire_error_deg: f32,
    pub settle_ms: f32,
    /// Scale on the view's turn rate and acceleration.
    pub turn_scale: f32,
    /// Full horizontal field of view perception probes inside.
    pub fov_deg: f32,
    /// Scale on every hearing range.
    pub hearing: f32,
    /// Side-steps while trading shots instead of standing still.
    pub strafes: bool,
    /// Reloads a part-used magazine when nothing is in sight.
    pub tactical_reload: bool,
}

impl BotSkill {
    pub fn for_level(level: SkillLevel, seed: u64) -> Self {
        // A stable per-bot factor in [0.85, 1.15].
        let unit = ((seed >> 17) & 0xffff) as f32 / 65535.0;
        let vary = 0.85 + 0.30 * unit;
        let base = match level {
            SkillLevel::Recruit => Self {
                level,
                reaction_ticks: 9,
                aim_error_deg: 3.0,
                acquire_error_deg: 9.0,
                settle_ms: 900.0,
                turn_scale: 0.7,
                fov_deg: 80.0,
                hearing: 0.6,
                strafes: false,
                tactical_reload: false,
            },
            SkillLevel::Regular => Self {
                level,
                reaction_ticks: 6,
                aim_error_deg: 1.6,
                acquire_error_deg: 6.0,
                settle_ms: 600.0,
                turn_scale: 1.0,
                fov_deg: 100.0,
                hearing: 1.0,
                strafes: true,
                tactical_reload: true,
            },
            SkillLevel::Hardened => Self {
                level,
                reaction_ticks: 4,
                aim_error_deg: 0.9,
                acquire_error_deg: 4.0,
                settle_ms: 400.0,
                turn_scale: 1.25,
                fov_deg: 110.0,
                hearing: 1.2,
                strafes: true,
                tactical_reload: true,
            },
            SkillLevel::Veteran => Self {
                level,
                reaction_ticks: 3,
                aim_error_deg: 0.45,
                acquire_error_deg: 2.5,
                settle_ms: 280.0,
                turn_scale: 1.5,
                fov_deg: 120.0,
                hearing: 1.4,
                strafes: true,
                tactical_reload: true,
            },
        };
        Self {
            reaction_ticks: ((base.reaction_ticks as f32 * vary).round() as u32).max(2),
            aim_error_deg: base.aim_error_deg * vary,
            acquire_error_deg: base.acquire_error_deg * vary,
            settle_ms: base.settle_ms * vary,
            ..base
        }
    }
}

impl Default for BotSkill {
    fn default() -> Self {
        Self::for_level(SkillLevel::default(), 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_cover_names_and_numbers() {
        for (index, level) in SkillLevel::ALL.into_iter().enumerate() {
            assert_eq!(SkillLevel::from_token(level.label()), Some(level));
            assert_eq!(SkillLevel::from_token(&index.to_string()), Some(level));
        }
        assert_eq!(
            SkillLevel::from_token(" Veteran "),
            Some(SkillLevel::Veteran)
        );
        assert_eq!(SkillLevel::from_token("godlike"), None);
    }

    #[test]
    fn each_level_is_strictly_sharper_than_the_one_below() {
        // Compared at the two ends of the per-bot variation, so the worst bot
        // of a level still outshoots the best bot of the level beneath it.
        for pair in SkillLevel::ALL.windows(2) {
            let low = BotSkill::for_level(pair[0], 0);
            let high = BotSkill::for_level(pair[1], u64::MAX);
            assert!(high.aim_error_deg < low.aim_error_deg, "{pair:?}");
            assert!(high.reaction_ticks <= low.reaction_ticks, "{pair:?}");
            assert!(high.turn_scale > low.turn_scale, "{pair:?}");
            assert!(high.fov_deg > low.fov_deg, "{pair:?}");
        }
    }
}
