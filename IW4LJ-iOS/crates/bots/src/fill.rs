//! Filling a hosted match with bots when it starts, so a player on their own
//! has a lobby without adding one by hand.

/// What the host asked for. `bot fill <off|auto|N>` sets it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillMode {
    Off,
    /// A lobby sized to the map.
    #[default]
    Auto,
    /// This many players in the match, the host included.
    Players(u32),
}

/// The most players a filled match holds, the host included.
pub const MAX_FILL_PLAYERS: u32 = 18;
/// Spawn points spread over less than this are a small map.
const SMALL_MAP_SPAN: f32 = 2400.0;

impl FillMode {
    pub fn from_token(token: &str) -> Option<Self> {
        match token.trim().to_ascii_lowercase().as_str() {
            "off" | "0" | "none" => Some(Self::Off),
            "auto" | "on" => Some(Self::Auto),
            other => other
                .parse::<u32>()
                .ok()
                .filter(|players| (2..=MAX_FILL_PLAYERS).contains(players))
                .map(Self::Players),
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Off => "off".to_owned(),
            Self::Auto => "auto".to_owned(),
            Self::Players(players) => players.to_string(),
        }
    }

    /// Bots to add as `(enemy, friendly)` for a host playing alone. A team
    /// mode splits the lobby evenly and gives the host's side one fewer bot;
    /// free-for-all has no friends.
    pub fn bots(self, team_based: bool, map_span: f32) -> (u32, u32) {
        let players = match self {
            Self::Off => return (0, 0),
            Self::Auto => lobby_size(team_based, map_span),
            Self::Players(players) => players.clamp(2, MAX_FILL_PLAYERS),
        };
        if team_based {
            let side = players.div_ceil(2);
            (side, side.saturating_sub(1))
        } else {
            (players - 1, 0)
        }
    }
}

/// The stock lobby sizes: twelve for team modes and eight for free-for-all,
/// stepped down on a map too small to hold them.
fn lobby_size(team_based: bool, map_span: f32) -> u32 {
    let small = map_span > 0.0 && map_span < SMALL_MAP_SPAN;
    match (team_based, small) {
        (true, false) => 12,
        (true, true) => 8,
        (false, false) => 8,
        (false, true) => 6,
    }
}

/// The larger horizontal extent of a map's spawn points: how big the playable
/// area is, near enough. Zero when the map has none.
pub fn spawn_span(spawns: &[[f32; 3]]) -> f32 {
    let extent = |axis: usize| {
        let (low, high) = spawns
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), spawn| {
                (low.min(spawn[axis]), high.max(spawn[axis]))
            });
        (high - low).max(0.0)
    };
    if spawns.is_empty() {
        0.0
    } else {
        extent(0).max(extent(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip_and_reject_nonsense() {
        for mode in [FillMode::Off, FillMode::Auto, FillMode::Players(10)] {
            assert_eq!(FillMode::from_token(&mode.label()), Some(mode));
        }
        assert_eq!(FillMode::from_token("on"), Some(FillMode::Auto));
        assert_eq!(
            FillMode::from_token("1"),
            None,
            "a lobby of one is no lobby"
        );
        assert_eq!(FillMode::from_token("40"), None);
        assert_eq!(FillMode::from_token("lots"), None);
    }

    #[test]
    fn a_filled_lobby_counts_the_host() {
        // Six a side: the host and five friends against six.
        assert_eq!(FillMode::Auto.bots(true, 5000.0), (6, 5));
        // Eight in a free-for-all: the host and seven others.
        assert_eq!(FillMode::Auto.bots(false, 5000.0), (7, 0));
        assert_eq!(FillMode::Auto.bots(true, 1500.0), (4, 3));
        assert_eq!(FillMode::Auto.bots(false, 1500.0), (5, 0));
        assert_eq!(FillMode::Players(18).bots(true, 1500.0), (9, 8));
        assert_eq!(FillMode::Players(7).bots(true, 0.0), (4, 3));
        assert_eq!(FillMode::Off.bots(true, 5000.0), (0, 0));
    }

    #[test]
    fn span_is_the_wider_axis_of_the_spawns() {
        assert_eq!(spawn_span(&[]), 0.0);
        let spawns = [[-100.0, 0.0, 5.0], [900.0, 300.0, 0.0], [0.0, -200.0, 9.0]];
        assert_eq!(spawn_span(&spawns), 1000.0);
    }
}
