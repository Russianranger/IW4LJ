#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathOutcome {
    None,
    Clear,
    Blocked,
    Unreachable,
    BudgetExhausted,
    IncompleteGraph,
    ProgressLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveMode {
    Hold,
    Walk,
    Drop,
    BreakGlass,
    Mantle,
    Ladder,
}

/// The widest the firing cone gets: point-blank, where turning is the hard part.
pub const DEFAULT_FIRE_CONE_DEG: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BotIntent {
    pub look_at: Option<[f32; 3]>,
    pub move_goal: Option<[f32; 3]>,
    pub desired_range: Option<f32>,
    pub move_mode: MoveMode,
    pub fire: bool,
    pub use_button: bool,
    pub reload: bool,
    pub crouch: bool,
    pub sprint: bool,
    /// Raise the sights.
    pub ads: bool,
    /// Hold fire until the sights are fully up: a scoped rifle from the hip
    /// hits nothing.
    pub ads_required: bool,
    pub melee: bool,
    /// One press of jump, for the low obstacle a walk alone does not clear.
    pub jump: bool,
    /// The trigger has to come up between shots: anything but full auto.
    pub tap_fire: bool,
    /// Fire in short bursts with a pause, for automatic fire at range.
    pub burst: bool,
    /// Half-angle, in degrees, the view must be inside before the trigger is
    /// pulled. Narrow at range, wide up close.
    pub fire_cone_deg: f32,
    /// The weapon the command asks for. `None` keeps the observed weapon; a
    /// selection is held through every phase of the change, not pulsed once.
    pub weapon: Option<u16>,
    pub path: PathOutcome,
}

impl Default for BotIntent {
    fn default() -> Self {
        Self {
            look_at: None,
            move_goal: None,
            desired_range: None,
            move_mode: MoveMode::Hold,
            fire: false,
            use_button: false,
            reload: false,
            crouch: false,
            sprint: false,
            ads: false,
            ads_required: false,
            melee: false,
            jump: false,
            tap_fire: false,
            burst: false,
            fire_cone_deg: DEFAULT_FIRE_CONE_DEG,
            weapon: None,
            path: PathOutcome::None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotorReport {
    #[default]
    Executing,
    Arrived,
    Blocked,
    Unsupported,
}
