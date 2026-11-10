use sim::{ClientId, ClientLifecycle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stance {
    Stand,
    Crouch,
    Prone,
}

impl Stance {
    pub fn from_view_height(height: f32) -> Self {
        if height >= 50.0 {
            Self::Stand
        } else if height >= 30.0 {
            Self::Crouch
        } else {
            Self::Prone
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnowledgeSource {
    CurrentlySeen,
    LastSeen,
    /// Placed by sound, the minimap or a UAV sweep: where, never a line of fire.
    Heard,
    PublicMode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub id: ClientId,
    pub origin: [f32; 3],
    pub source: KnowledgeSource,
    pub seen_tick: u32,
    pub confidence: f32,
    /// The contact's eye height when it was observed; `0.0` when unknown.
    pub height: f32,
}

impl Contact {
    /// Where to put the sights: the upper chest of whatever stance the contact
    /// was seen in, so a crouched or prone enemy is not shot over.
    pub fn aim_point(&self) -> [f32; 3] {
        let height = if self.height > 1.0 { self.height } else { 60.0 };
        [
            self.origin[0],
            self.origin[1],
            self.origin[2] + (height * 0.8).clamp(10.0, 52.0),
        ]
    }
}

/// What perception established about one client this tick. `NotSeen` is an
/// answer; `Unknown` is the absence of one, and the two must not be confused —
/// a probe that was never performed cannot end a contact or authorize a shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Seen,
    NotSeen,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WeaponClass {
    #[default]
    Assault,
    Smg,
    Lmg,
    Sniper,
    Shotgun,
    Pistol,
    Launcher,
}

impl WeaponClass {
    /// The range this weapon wants to fight at. Inside it the bot stops closing
    /// and trades shots; firing positions are picked in a band around it.
    pub fn fight_hold(self) -> f32 {
        match self {
            Self::Sniper => 1500.0,
            Self::Lmg => 800.0,
            Self::Assault => 700.0,
            Self::Launcher => 600.0,
            Self::Smg => 420.0,
            Self::Pistol => 380.0,
            Self::Shotgun => 220.0,
        }
    }

    /// The furthest range worth standing and shooting from. Past it the weapon
    /// is not winning the exchange, so the bot closes the distance instead.
    pub fn engage_max(self) -> f32 {
        match self {
            Self::Sniper => 6000.0,
            Self::Lmg => 2200.0,
            Self::Assault => 1800.0,
            Self::Launcher => 1500.0,
            Self::Smg => 1100.0,
            Self::Pistol => 900.0,
            Self::Shotgun => 450.0,
        }
    }

    /// Range beyond which the sights come up. Closer than this the hip is
    /// faster; a shotgun never aims, a sniper always does.
    pub fn ads_min(self) -> f32 {
        match self {
            Self::Sniper | Self::Launcher => 0.0,
            Self::Assault => 300.0,
            Self::Lmg => 350.0,
            Self::Pistol => 380.0,
            Self::Smg => 420.0,
            Self::Shotgun => f32::INFINITY,
        }
    }

    /// The engine's own `weapClass`, which every weapon carries. Offhands,
    /// turrets and items are not fighting classes and return `None`.
    pub fn from_weap_class(weap_class: i32) -> Option<Self> {
        Some(match weap_class {
            0 => Self::Assault,
            1 => Self::Sniper,
            2 => Self::Lmg,
            3 => Self::Smg,
            4 => Self::Shotgun,
            5 => Self::Pistol,
            7 => Self::Launcher,
            _ => return None,
        })
    }

    pub fn from_script_name(name: &str) -> Self {
        if name.is_empty() {
            return Self::Assault;
        }
        let stem = crate::unique_loadout::family_stem(name);
        for row in crate::unique_loadout::UNIQUE_WEAPONS {
            if crate::unique_loadout::family_stem(row.key) == stem {
                return Self::from_group(row.group);
            }
        }
        Self::Assault
    }

    fn from_group(group: crate::unique_loadout::UniqueGroup) -> Self {
        use crate::unique_loadout::UniqueGroup;
        match group {
            UniqueGroup::Sniper => Self::Sniper,
            UniqueGroup::Lmg => Self::Lmg,
            UniqueGroup::Shotgun => Self::Shotgun,
            UniqueGroup::Smg | UniqueGroup::MachinePistol => Self::Smg,
            UniqueGroup::Pistol => Self::Pistol,
            UniqueGroup::Projectile => Self::Launcher,
            UniqueGroup::Assault | UniqueGroup::Riot | UniqueGroup::Cqb | UniqueGroup::Special => {
                Self::Assault
            }
        }
    }
}

/// What the weapon simulator is doing with the active hand. The bot reads it;
/// it never reimplements the transitions behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WeaponAction {
    #[default]
    Ready,
    Raising,
    Dropping,
    Firing,
    Reloading,
    /// Melee, offhand, rechamber, sprint or a stun — not a readiness decision.
    Busy,
}

impl WeaponAction {
    pub fn from_weaponstate(raw: i32) -> Self {
        use weapon_iw4::WeaponState as State;
        match State::from_i32(raw) {
            Ok(State::Ready) => Self::Ready,
            Ok(State::Raising | State::RaisingAltswitch) => Self::Raising,
            Ok(State::Dropping | State::DroppingQuick | State::DroppingAltswitch) => Self::Dropping,
            Ok(State::Firing) => Self::Firing,
            Ok(state) if state.is_reload_family() => Self::Reloading,
            _ => Self::Busy,
        }
    }

    /// An active weapon id alone does not mean the hand finished raising.
    pub fn is_settled(self) -> bool {
        matches!(self, Self::Ready | Self::Firing)
    }
}

/// Timing and magazine facts for one weapon id, read from the content the
/// simulator uses. Not a second weapon model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponFacts {
    /// A carried gun the player can select rather than an offhand or an item.
    pub selectable: bool,
    /// Pistol-class selection uses the quick raise and drop timings.
    pub quick_select: bool,
    pub clip_size: i32,
    pub raise_time_ms: i32,
    pub quick_raise_time_ms: i32,
    pub drop_time_ms: i32,
    pub quick_drop_time_ms: i32,
    pub reload_time_ms: i32,
    pub reload_empty_time_ms: i32,
    /// Anything but full auto: the trigger has to come up between shots (or
    /// bursts), so a held button fires once.
    pub fire_single: bool,
    pub fire_time_ms: i32,
    pub bolt_action: bool,
    /// The weapon has sights to raise.
    pub can_ads: bool,
}

/// One owned weapon with the ammunition the match state holds for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponSlot {
    pub weapon: u16,
    pub class: WeaponClass,
    pub facts: WeaponFacts,
    pub clip: i32,
    pub stock: i32,
}

impl WeaponSlot {
    /// Milliseconds from committing to a change until this weapon is raised,
    /// leaving `from` behind. Both halves follow the incoming weapon's class.
    pub fn switch_ms(&self, from: &Self) -> i32 {
        if self.facts.quick_select {
            from.facts.quick_drop_time_ms + self.facts.quick_raise_time_ms
        } else {
            from.facts.drop_time_ms + self.facts.raise_time_ms
        }
    }

    pub fn reload_ms(&self) -> i32 {
        if self.clip == 0 {
            self.facts.reload_empty_time_ms
        } else {
            self.facts.reload_time_ms
        }
    }

    pub fn is_loaded_gun(&self) -> bool {
        self.facts.selectable && self.clip > 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelfState {
    pub life_sequence: sim::LifeSequence,
    pub id: ClientId,
    pub lifecycle: ClientLifecycle,
    pub origin: [f32; 3],
    pub viewangles: [f32; 3],
    pub delta_angles: [f32; 3],
    pub view_height: f32,
    pub stance: Stance,
    pub health: i32,
    pub weapon: u16,
    pub weapon_class: WeaponClass,
    pub weapon_action: WeaponAction,
    pub ammo_clip: i32,
    pub ammo_stock: i32,
    pub team: i32,
    pub velocity: [f32; 3],
    /// How far the sights are up, 0 at the hip to 1 fully aimed.
    pub ads: f32,
    /// Counts up each time the bot takes damage.
    pub damage_event: u32,
    /// World yaw from the bot toward whatever last hurt it, when the hit had a
    /// direction. The same fact the damage indicator draws for a player.
    pub attacker_yaw: Option<f32>,
    /// The bot's team has a radar sweep running.
    pub radar: bool,
}

pub const DEFAULT_OBJECTIVE_RADIUS: f32 = 96.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ObjectiveAction {
    #[default]
    Capture,
    Plant,
    Defuse,
    Defend,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TeamRole {
    #[default]
    Actor,
    Cover,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModeObjective {
    pub id: u32,
    pub round: u32,
    pub action: ObjectiveAction,
    pub role: TeamRole,
    pub active_user: Option<ClientId>,
    pub remaining_ms: Option<u32>,
    pub interaction_ms: u32,
    pub progress: f32,
    pub origin: [f32; 3],
    pub touching: bool,
    pub use_button: bool,
    pub radius: f32,
}

impl ModeObjective {
    pub fn at(origin: [f32; 3]) -> Self {
        Self {
            id: 0,
            round: 0,
            action: ObjectiveAction::Capture,
            role: TeamRole::Actor,
            active_user: None,
            remaining_ms: None,
            interaction_ms: 0,
            progress: 0.0,
            origin,
            touching: false,
            use_button: false,
            radius: DEFAULT_OBJECTIVE_RADIUS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotEvent {
    Spotted { id: ClientId },
    LostSight { id: ClientId },
}

#[derive(Clone, Debug, PartialEq)]
pub struct BotObservation {
    pub tick: u32,
    pub time_ms: i32,
    pub self_state: SelfState,
    /// The bot's own carried weapons. Enemy inventories are never projected.
    pub inventory: Vec<WeaponSlot>,
    pub seen: Vec<Contact>,
    /// Enemies placed without a line of sight: unsuppressed gunfire, footsteps
    /// close by, a UAV sweep. A position to go and look at, never a target.
    pub heard: Vec<Contact>,
    /// Enemies that are in the match but not alive. A remembered position for
    /// one of them is the position of a body.
    pub gone: Vec<ClientId>,
    /// Clients perception did not finish evaluating this tick, because its
    /// allowance ran out, a query was denied, or a trace came back
    /// unclassified. A client in neither `seen` nor here was evaluated and not
    /// seen, which is a real negative observation.
    pub unsensed: Vec<ClientId>,
    pub events: Vec<BotEvent>,
    pub objective: Option<ModeObjective>,
    pub objectives: Vec<ModeObjective>,
}

impl BotObservation {
    pub fn held(&self) -> Option<&WeaponSlot> {
        self.slot(self.self_state.weapon)
    }

    pub fn slot(&self, weapon: u16) -> Option<&WeaponSlot> {
        self.inventory.iter().find(|slot| slot.weapon == weapon)
    }

    pub fn owns(&self, weapon: u16) -> bool {
        self.inventory.iter().any(|slot| slot.weapon == weapon)
    }

    pub fn eye(&self) -> [f32; 3] {
        let z = if self.self_state.view_height > 1.0 {
            self.self_state.view_height
        } else {
            60.0
        };
        [
            self.self_state.origin[0],
            self.self_state.origin[1],
            self.self_state.origin[2] + z,
        ]
    }

    pub fn visibility(&self, id: ClientId) -> Visibility {
        if self.seen.iter().any(|contact| contact.id == id) {
            Visibility::Seen
        } else if self.unsensed.contains(&id) {
            Visibility::Unknown
        } else {
            Visibility::NotSeen
        }
    }

    pub fn events_since(&self, prev_seen: &[ClientId]) -> Vec<BotEvent> {
        let mut events = Vec::new();
        for contact in &self.seen {
            if !prev_seen.contains(&contact.id) {
                events.push(BotEvent::Spotted { id: contact.id });
            }
        }
        for id in prev_seen {
            // Only a completed negative observation is a loss of sight. A skipped
            // probe leaves the previous knowledge exactly as it was.
            if self.visibility(*id) == Visibility::NotSeen {
                events.push(BotEvent::LostSight { id: *id });
            }
        }
        events
    }
}
