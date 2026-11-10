use playerstate_iw4::{UserCmd, buttons};
use sim::{ClientId, ClientLifecycle};

use crate::grenade;
use crate::intent::{BotIntent, DEFAULT_FIRE_CONE_DEG, MotorReport, MoveMode, PathOutcome};
use crate::memory::Memory;
use crate::motor::Motor;
use crate::nav::{self, NavGraph, PathError};
use crate::observation::{
    BotObservation, Contact, KnowledgeSource, ModeObjective, ObjectiveAction, Offhand, StreakKind,
    TeamRole, ThrowKind, WeaponAction,
};
use crate::query::{QueryResult, QuerySubsystem, SightSample, WalkSample, WorldQuery};
use crate::skill::{BotSkill, SkillLevel};
use crate::task::{ActionStage, Decision, SwitchReason, Task, TaskKind};
use crate::weapon::WeaponSkill;

const SWITCH_MARGIN: f32 = 12.0;
const STEP_IN: f32 = 64.0;
const WAYPOINT_IN: f32 = 24.0;
const LAST_SEEN_FRESH_TICKS: u32 = 40;
/// How stale the positive observation behind a shot may be. Committing to a
/// target is a movement and aiming decision; firing needs the enemy to have
/// been seen, not merely remembered.
const FIRE_FRESH_TICKS: u32 = 4;
/// Free-space fallback ring, used only where there is no navigation graph.
const FIGHT_RING: u32 = 12;
/// How many baked supports one scan evaluates. Two queries each, so the whole
/// scan fits inside a few slices of the shared quota.
const FIGHT_CANDIDATES: usize = 6;
/// How far the threat may move before its firing position is re-derived. Far
/// wider than a step, so a moving target does not rebuild the goal every tick.
const FIGHT_COMMIT_MOVE: f32 = 128.0;
const FIGHT_SCAN_MAX_AGE_TICKS: u32 = 8;
/// A committed position is abandoned if the engagement has not resolved by
/// then, so a stale goal cannot outlive its fight.
const FIGHT_COMMIT_MAX_AGE_TICKS: u32 = 60;
/// Commanded movement without displacement for this long is a stuck bot.
const STUCK_TICKS: u32 = 40;
/// How long the last place an enemy was seen or heard stays worth walking to.
const HOTSPOT_TTL_TICKS: u32 = 400;
/// Newer word of an enemy has to beat the current roam goal's by this much
/// before the bot turns round for it.
const HOTSPOT_REDIRECT_TICKS: u32 = 20;
/// How long a place or a contact the bot found no route to is left alone.
/// Without it an enemy heard from somewhere unreachable is walked at, given up
/// on and walked at again for as long as the noise lasts.
const NO_ROUTE_BACKOFF_TICKS: u32 = 200;
/// How long a bot faces the direction it was hurt from, and how far out along
/// that bearing it looks. Sight then has to find the attacker; nothing here
/// knows who or where they are.
const ALERT_TICKS: u32 = 30;
const ALERT_LOOK_IN: f32 = 600.0;
/// Route points ahead of the bot it looks toward while travelling, so it faces
/// round a corner before it turns it.
const LOOK_AHEAD_STEPS: usize = 4;
/// Inside this the knife is faster than the gun.
const MELEE_RANGE_IN: f32 = 56.0;
/// Automatic fire past this range goes out in bursts.
const BURST_RANGE_IN: f32 = 900.0;
/// One side-step of a strafe, and how long a leg lasts before the next.
const STRAFE_STEP_IN: f32 = 72.0;
const STRAFE_LEG_TICKS: u32 = 10;
/// A grenade is thrown at a position this far out: nearer and the thrower is
/// inside its own blast, further and it does not arrive.
const GRENADE_MIN_IN: f32 = 380.0;
const GRENADE_MAX_IN: f32 = 1150.0;
/// How fresh word of an enemy has to be to be worth a grenade.
const GRENADE_FRESH_TICKS: u32 = 24;
/// The pin is held this long, and the arm is given this long to finish the
/// throw after it is let go.
const GRENADE_HOLD_TICKS: u32 = 6;
const GRENADE_FOLLOW_TICKS: u32 = 14;
/// The view gets this long to reach the throwing angle before the throw goes
/// anyway.
const GRENADE_AIM_TICKS: u32 = 16;
/// The shortest gap between one bot's throws; a random stretch is added.
const GRENADE_COOLDOWN_TICKS: u32 = 160;
/// How long a killstreak gets to be called in before the bot stops trying and
/// waits to try again.
const STREAK_TICKS: u32 = 120;
const STREAK_RETRY_TICKS: u32 = 300;
/// Holding use on something for this long without it finishing means it is not
/// going to; the objective is then left alone for a while.
const USE_GIVE_UP_TICKS: u32 = 240;
const USE_SHUN_TICKS: u32 = 900;

#[derive(Clone, Debug)]
pub struct HostController {
    seed: u64,
    rng: u64,
    skill: BotSkill,
    aggression: f32,
    self_preserve: f32,
    memory: Memory,
    task: Task,
    intent: BotIntent,
    motor: Motor,
    last_think_tick: Option<u32>,
    reaction_until: u32,
    last_target: Option<ClientId>,
    /// The client a live fire authorization rests on, and the tick of the
    /// positive observation that granted it.
    fire_target: Option<(ClientId, u32)>,
    committed: Vec<nav::RouteStep>,
    route_goal: Option<[f32; 3]>,
    route_work: nav::RouteWork,
    fight_scan: Option<FightScan>,
    weapon: WeaponSkill,
    nav_key: Option<(u64, u64, u32, u32)>,
    roam: Option<[f32; 3]>,
    roam_gen: u32,
    /// The tick of the knowledge the current roam goal came from; `None` for a
    /// goal picked at random.
    roam_tick: Option<u32>,
    /// The current roam goal is a hotspot and not a random node.
    roam_hot: bool,
    /// The last place an enemy was seen or heard, and when. Where a bot with
    /// nothing in sight goes to look.
    hotspot: Option<([f32; 3], u32)>,
    /// Whose position the hotspot is, so it can be dropped when they die.
    hotspot_of: Option<ClientId>,
    /// The remembered contact the investigate task walks to.
    investigate: Option<Contact>,
    /// A contact no route was found to, and the tick it may be tried again.
    investigate_block: Option<(ClientId, u32)>,
    /// A point to face after being hurt from out of sight, and when it lapses.
    alert: Option<([f32; 3], u32)>,
    damage_event: Option<u32>,
    /// The live strafe leg: where it ends and the tick it is abandoned.
    strafe: Option<([f32; 3], u32)>,
    strafe_side: f32,
    /// The throw in progress.
    grenade: Option<GrenadeAct>,
    grenade_ready_tick: u32,
    /// Throws made this life, lethal then tactical.
    thrown: [u8; 2],
    /// The killstreak being called in, and the tick the attempt is dropped.
    streak: Option<(u16, StreakKind, u32)>,
    streak_ready_tick: u32,
    /// The use objective being held, and since when.
    using: Option<(u32, u32)>,
    /// A use objective given up on, and the tick it may be tried again.
    shunned: Option<(u32, u32)>,
    decision: Decision,
    was_touching: bool,
    last_cmd: UserCmd,
    life_sequence: Option<sim::LifeSequence>,
    report: MotorReport,
    progress_origin: Option<[f32; 3]>,
    progress_tick: u32,
    escape: Option<([f32; 3], u32)>,
}

impl HostController {
    pub fn new(seed: u64) -> Self {
        Self::with_skill(seed, SkillLevel::default())
    }

    pub fn with_skill(seed: u64, level: SkillLevel) -> Self {
        let aggression = ((seed & 0xff) as f32) / 255.0;
        let self_preserve = (((seed >> 8) & 0xff) as f32) / 255.0;
        let skill = BotSkill::for_level(level, seed);
        Self {
            seed,
            // xorshift has no zero state.
            rng: seed.rotate_left(29) | 1,
            skill,
            aggression,
            self_preserve,
            memory: Memory::default(),
            task: Task::default(),
            intent: BotIntent::default(),
            motor: Motor::with_skill(seed, skill),
            last_think_tick: None,
            reaction_until: 0,
            last_target: None,
            fire_target: None,
            committed: Vec::new(),
            route_goal: None,
            route_work: nav::RouteWork::default(),
            fight_scan: None,
            weapon: WeaponSkill::default(),
            nav_key: None,
            roam: None,
            roam_gen: 0,
            roam_tick: None,
            roam_hot: false,
            hotspot: None,
            hotspot_of: None,
            investigate: None,
            investigate_block: None,
            alert: None,
            damage_event: None,
            strafe: None,
            strafe_side: if seed & 0x1_0000 == 0 { 1.0 } else { -1.0 },
            grenade: None,
            grenade_ready_tick: 0,
            thrown: [0; 2],
            streak: None,
            streak_ready_tick: 0,
            using: None,
            shunned: None,
            decision: Decision::default(),
            was_touching: false,
            last_cmd: UserCmd::default(),
            life_sequence: None,
            report: MotorReport::Executing,
            progress_origin: None,
            progress_tick: 0,
            escape: None,
        }
    }

    pub fn skill(&self) -> &BotSkill {
        &self.skill
    }

    /// Moves the bot to another difficulty, keeping its own variation of it.
    pub fn set_skill_level(&mut self, level: SkillLevel) {
        if self.skill.level != level {
            self.skill = BotSkill::for_level(level, self.seed);
            self.motor.set_skill(self.skill);
        }
    }

    /// The buttons to let go on a tick the bot does not think on, when its
    /// last command is sent again. They only act on a fresh press: a
    /// semi-automatic weapon fires once per pull, so a trigger held through
    /// the gap would fire one shot per engagement.
    pub fn released_between_thinks(&self) -> u32 {
        let mut released = buttons::JUMP | buttons::MELEE_CHARGE | buttons::LOCATION_SELECT;
        if self.intent.tap_fire || self.intent.place_held {
            released |= buttons::ATTACK;
        }
        released
    }

    pub(crate) fn cancel_navigation(&mut self) {
        self.route_work.cancel();
        self.fight_scan = None;
        self.committed.clear();
        self.route_goal = None;
        self.strafe = None;
    }

    pub fn decision(&self) -> Decision {
        self.decision
    }

    pub fn route_stats(&self) -> nav::RouteStats {
        self.route_work.stats()
    }

    /// Slices the live route request has waited, and its longest run without
    /// granted progress. `(0, 0)` when nothing is pending.
    pub fn route_age(&self) -> (u32, u32) {
        self.route_work.live_age()
    }

    /// A bot with a movement task that has not moved. Holding, interacting and
    /// waiting to respawn are not navigation failures.
    pub fn stuck(&self, tick: u32) -> bool {
        matches!(
            self.intent.move_mode,
            MoveMode::Walk | MoveMode::Drop | MoveMode::BreakGlass
        ) && tick.saturating_sub(self.progress_tick) >= STUCK_TICKS
    }

    /// The enemy this bot has committed to, for perception to refresh before it
    /// spends the same allowance looking for new ones. `None` when the bot is
    /// not fighting.
    pub fn focus_target(&self) -> Option<ClientId> {
        self.last_target
            .filter(|_| self.task.kind == TaskKind::Fight)
    }

    pub fn task(&self) -> Task {
        self.task
    }

    pub fn intent(&self) -> BotIntent {
        self.intent
    }

    pub fn report(&self) -> MotorReport {
        self.report
    }

    pub fn last_cmd(&self) -> UserCmd {
        self.last_cmd
    }

    pub fn drive(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        dt_ms: i32,
    ) -> UserCmd {
        self.drive_nav(obs, world, None, &mut 512, dt_ms)
    }

    pub fn drive_nav(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        dt_ms: i32,
    ) -> UserCmd {
        world.enter(QuerySubsystem::Execution);
        let key = nav.map(|g| (g.generation, g.digest, g.schema, g.hull));
        if self.nav_key != key {
            self.nav_key = key;
            self.cancel_navigation();
        }
        if self.life_sequence != Some(obs.self_state.life_sequence) {
            self.life_sequence = Some(obs.self_state.life_sequence);
            self.cancel_navigation();
            self.weapon.reset();
            self.roam = None;
            self.roam_tick = None;
            self.roam_hot = false;
            self.hotspot = None;
            self.hotspot_of = None;
            self.investigate = None;
            self.investigate_block = None;
            self.alert = None;
            self.damage_event = None;
            self.grenade = None;
            self.thrown = [0; 2];
            self.streak = None;
            self.using = None;
            self.memory = Memory::default();
            self.was_touching = false;
            self.last_think_tick = None;
            self.last_target = None;
            self.fire_target = None;
            self.progress_origin = None;
            self.escape = None;
            self.decision = Decision::default();
            self.task = Task::default();
        }
        if obs.self_state.lifecycle != ClientLifecycle::Alive {
            self.task = Task {
                kind: TaskKind::Hunt,
                reason: SwitchReason::Died,
            };
            self.intent = BotIntent::default();
            self.route_work.cancel();
            self.weapon.reset();
            self.fight_scan = None;
            self.fire_target = None;
            self.decision.stage = ActionStage::Failed;
            self.decision.objective = None;
            self.committed.clear();
            self.last_cmd = UserCmd {
                server_time: obs.time_ms,
                weapon: obs.self_state.weapon,
                weapon_mapped: obs.self_state.weapon,
                ..UserCmd::default()
            };
            return self.last_cmd;
        }
        // The host already spaces the calls out; every bot thinks on each one.
        // Staggering by client id on top of that left the ids of one parity
        // with an intent that was only ever refreshed by a new sighting.
        if self.last_think_tick != Some(obs.tick) {
            self.think(obs, world, nav, astar_budget);
            self.last_think_tick = Some(obs.tick);
        }
        // Intents are reused between scheduled thinks, so the authorization is
        // re-checked on the tick that actually emits the command.
        self.gate_fire(obs);
        if self.intent.move_mode == MoveMode::BreakGlass {
            self.intent.fire = false;
            self.intent.sprint = false;
            self.intent.use_button = false;
            if let Some(goal) = self.intent.move_goal {
                world.enter(QuerySubsystem::Tactical);
                let eye = obs.eye();
                // Try several body-height points on the obstructed corridor.
                for height in [obs.self_state.view_height.max(40.0), 40.0, 20.0] {
                    let target = [goal[0], goal[1], goal[2] + height];
                    if let Ok(SightSample::Blocked {
                        fraction,
                        obstacle: crate::query::ObstacleKind::Glass,
                    }) = world.shot_ray(eye, target, obs.self_state.id)
                    {
                        self.intent.look_at = Some(std::array::from_fn(|i| {
                            eye[i] + (target[i] - eye[i]) * fraction
                        }));
                        self.intent.fire = obs.self_state.ammo_clip > 0
                            && self.intent.weapon.is_none()
                            && !self.intent.reload
                            && (self.task.kind != TaskKind::Fight
                                || obs.tick >= self.reaction_until);
                        break;
                    }
                }
            }
        }
        let cmd = self.motor.drive(obs, &self.intent, dt_ms);
        self.report = self.motor.report(obs, &self.intent, &cmd);
        self.last_cmd = cmd;
        cmd
    }

    /// A retained target is never a standing permission to shoot: the shot has
    /// to stay backed by a positive observation of that same client, refreshed
    /// on every tick perception actually looked.
    fn gate_fire(&mut self, obs: &BotObservation) {
        if !self.intent.fire {
            return;
        }
        let Some((id, since)) = self.fire_target.as_mut() else {
            self.intent.fire = false;
            return;
        };
        if obs.seen.iter().any(|contact| contact.id == *id) {
            *since = obs.tick;
        }
        if obs.tick.saturating_sub(*since) > FIRE_FRESH_TICKS {
            self.intent.fire = false;
        }
    }

    /// Records what a shot decision rests on. `None` withdraws the
    /// authorization, which the emitted-command path then enforces.
    fn authorize_fire(&mut self, fire: bool, target: Contact) -> bool {
        self.fire_target = fire.then_some((target.id, target.seen_tick));
        fire
    }

    fn think(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
    ) {
        let started = std::time::Instant::now();
        if self.shunned.is_some_and(|(_, until)| obs.tick >= until) {
            self.shunned = None;
        }
        let pruned;
        let obs = match self.shunned {
            Some((id, _)) => {
                pruned = obs.without_objective(id);
                &pruned
            }
            None => obs,
        };
        let events = self.memory.ingest(obs);
        self.note_threats(obs);
        let touching_use = obs
            .objectives
            .iter()
            .copied()
            .chain(obs.objective)
            .find(|obj| obj.touching && obj.use_button);
        if touching_use.is_none()
            && let Some(intent) = self.unstick(obs, world)
        {
            self.intent = intent;
            self.task.reason = SwitchReason::ProgressLost;
            self.decision.stage = ActionStage::Approach;
            self.apply_weapon_skill(obs);
            return;
        }
        let mut objective = touching_use.or_else(|| {
            obs.objectives
                .iter()
                .copied()
                .chain(obs.objective)
                .max_by(|a, b| {
                    objective_utility(obs, *a, None)
                        .total_cmp(&objective_utility(obs, *b, None))
                        .then_with(|| b.id.cmp(&a.id))
                })
        });
        let mut next = self.pick_task(obs, objective);
        if next.kind == TaskKind::TouchObj && touching_use.is_none() {
            objective = choose_objective(
                obs,
                world,
                nav,
                astar_budget,
                &mut self.committed,
                &mut self.route_goal,
                &mut self.route_work,
            );
            next = self.pick_task(obs, objective);
        }
        let switched = next.kind != self.task.kind;
        // Routes are keyed by destination. A temporary combat fallback must not
        // discard navigation progress on every think tick.
        self.task = next;
        let previous_stage = self.decision.stage;
        let previous = self.decision.objective;
        self.decision.objective = objective.filter(|_| self.task.kind == TaskKind::TouchObj);
        self.intent = self.intent_for(obs, world, nav, astar_budget, objective);
        self.decision.stage = if matches!(
            self.intent.path,
            PathOutcome::Blocked | PathOutcome::Unreachable
        ) {
            ActionStage::Failed
        } else if previous.is_some_and(|old| {
            old.use_button
                && obs
                    .objectives
                    .iter()
                    .chain(obs.objective.iter())
                    .any(|new| {
                        old.id == new.id
                            && old.round == new.round
                            && old.action != new.action
                            && new.action == ObjectiveAction::Defend
                    })
        }) {
            ActionStage::Complete
        } else if self.intent.use_button {
            ActionStage::Interact
        } else if self.intent.move_mode == MoveMode::Hold {
            ActionStage::Hold
        } else if objective
            .is_some_and(|o| dist2(obs.self_state.origin, o.origin) < o.radius * o.radius)
        {
            ActionStage::Position
        } else {
            ActionStage::Approach
        };
        self.intent.sprint = self.intent.move_mode == MoveMode::Walk
            && self.intent.path == PathOutcome::Clear
            && self.memory.currently_seen().next().is_none()
            && !self.intent.use_button
            && !self.intent.fire
            && !self.intent.reload
            && !self.intent.crouch
            && self
                .intent
                .move_goal
                .is_some_and(|g| dist_xy(obs.self_state.origin, g) > 160.0);
        // Standing clearance also fits a crouched hull. Require a shot at crouched eye height.
        if self.intent.fire
            && self.intent.move_mode == MoveMode::Hold
            && obs.self_state.health < 60
            && let Some(target) = self.pick_fight_target(obs)
            && world.walk_hull(obs.self_state.origin, obs.self_state.origin)
                == Ok(WalkSample::Clear)
        {
            let mut eye = obs.self_state.origin;
            eye[2] += 40.0;
            self.intent.crouch = shot_allowed(
                world.shot_ray(eye, self.intent.look_at.unwrap(), obs.self_state.id),
                target.id,
            );
        }
        if (self.intent.path == PathOutcome::Unreachable
            || self.intent.path == PathOutcome::Blocked)
            && (self.task.kind == TaskKind::TouchObj || self.task.kind == TaskKind::Investigate)
        {
            self.task.reason = SwitchReason::PathFailed;
        }
        let touching_now = objective.is_some_and(|obj| obj.touching);
        if self.task.kind == TaskKind::TouchObj
            && self.was_touching
            && !touching_now
            && self.decision.stage != ActionStage::Complete
        {
            self.intent.path = PathOutcome::ProgressLost;
            self.task.reason = SwitchReason::ProgressLost;
        }
        self.was_touching = touching_now && self.task.kind == TaskKind::TouchObj;
        // Hurt from somewhere it cannot see: face the bearing the damage came
        // from and let sight do the rest. An interaction is not turned away from.
        if let Some((point, _)) = self.alert
            && !self.intent.use_button
        {
            self.intent.look_at = Some(point);
            self.intent.sprint = false;
        }
        self.watch_use(obs, touching_use);
        self.apply_weapon_skill(obs);
        self.throw_grenade(obs, world);
        self.call_streak(obs);
        let weapon_act = self.weapon.act();
        if switched || previous_stage != self.decision.stage {
            let left = self
                .intent
                .move_goal
                .map(|goal| dist2(obs.self_state.origin, goal).sqrt())
                .unwrap_or(0.0);
            let know = self
                .memory
                .currently_seen()
                .next()
                .or_else(|| self.memory.last_seen().next())
                .map(|c| c.source);
            diag::info!(
                Sim,
                "bots: id={} tick={} task={:?} reason={:?} path={:?} know={know:?} origin=({:.0},{:.0},{:.0}) left={left:.0} events={events:?} decision={:?} weapon={weapon_act:?} think_us={} report={:?}",
                obs.self_state.id.0,
                obs.tick,
                self.task.kind,
                self.task.reason,
                self.intent.path,
                obs.self_state.origin[0],
                obs.self_state.origin[1],
                obs.self_state.origin[2],
                self.decision,
                started.elapsed().as_micros(),
                self.report,
            );
        }
    }

    /// The weapon channel has one owner. The skill commits to reloading or
    /// switching and keeps its request until the simulation confirms the
    /// outcome; movement and looking stay with the task.
    fn apply_weapon_skill(&mut self, obs: &BotObservation) {
        // An interaction already under way is not displaced by an empty magazine.
        let tactical = self.skill.tactical_reload
            && matches!(self.task.kind, TaskKind::Hunt | TaskKind::TouchObj)
            && self.alert.is_none();
        self.weapon.update(obs, !self.intent.use_button, tactical);
        self.intent.weapon = self.weapon.requested_weapon();
        self.intent.reload = self.weapon.wants_reload();
        if self.intent.weapon.is_some() {
            self.intent.fire = false;
        }
        if self.intent.weapon.is_some() || self.intent.reload {
            self.intent.sprint = false;
            self.intent.ads = false;
            self.intent.melee = false;
        }
    }

    /// Gives up on a use objective that has been held far longer than any of
    /// them takes: a crate on a roof, a prompt the bot is not quite facing.
    fn watch_use(&mut self, obs: &BotObservation, held: Option<ModeObjective>) {
        let Some(objective) = held.filter(|_| self.intent.use_button) else {
            self.using = None;
            return;
        };
        match self.using {
            Some((id, since)) if id == objective.id => {
                if obs.tick.saturating_sub(since) > USE_GIVE_UP_TICKS {
                    self.shunned = Some((id, obs.tick.saturating_add(USE_SHUN_TICKS)));
                    self.using = None;
                    self.intent.use_button = false;
                }
            }
            _ => self.using = Some((objective.id, obs.tick)),
        }
    }

    /// Where a grenade would do some good: an enemy placed a moment ago that
    /// the bot is not, right now, shooting at.
    fn grenade_target(&self, obs: &BotObservation) -> Option<[f32; 3]> {
        let fresh =
            |contact: &&Contact| obs.tick.saturating_sub(contact.seen_tick) <= GRENADE_FRESH_TICKS;
        let behind_cover = self
            .memory
            .last_seen()
            .filter(fresh)
            .max_by_key(|contact| contact.seen_tick);
        // In sight but with no shot for a while: dug in behind something.
        let no_shot = self
            .memory
            .currently_seen()
            .find(|_| !self.intent.fire && obs.tick > self.reaction_until.saturating_add(20));
        behind_cover.or(no_shot).map(|contact| contact.origin)
    }

    /// Starts a throw when there is a target for one, and carries a started
    /// throw through: turn to the angle, pull the pin, let go, follow through.
    fn throw_grenade(&mut self, obs: &BotObservation, world: &mut impl WorldQuery) {
        if let Some(act) = self.grenade {
            self.continue_throw(obs, act);
            return;
        }
        if self.skill.level == SkillLevel::Recruit
            || obs.tick < self.grenade_ready_tick
            || obs.self_state.weapon_action != WeaponAction::Ready
            || self.intent.use_button
            || self.intent.reload
            || self.intent.weapon.is_some()
            || self.streak.is_some()
            || !matches!(self.task.kind, TaskKind::Fight | TaskKind::Investigate)
        {
            return;
        }
        let Some(target) = self.grenade_target(obs) else {
            return;
        };
        let range = dist2(obs.self_state.origin, target).sqrt();
        if !(GRENADE_MIN_IN..=GRENADE_MAX_IN).contains(&range) {
            return;
        }
        // Whatever comes of it, the question is not asked again straight away.
        self.grenade_ready_tick = obs.tick.saturating_add(40);
        let usable = |slot: Option<Offhand>, made: u8, limit: u8| {
            slot.filter(|offhand| made < limit && offhand.facts.kind != ThrowKind::Other)
        };
        let (offhand, tactical) = match (
            usable(obs.self_state.lethal, self.thrown[0], 1),
            usable(obs.self_state.tactical, self.thrown[1], 2),
        ) {
            (Some(lethal), _) => (lethal, false),
            (None, Some(tactical)) => (tactical, true),
            (None, None) => return,
        };
        // Not every chance is taken: a bot that answers every sound with a
        // grenade has none left for the one that matters.
        if self.next_unit() > 0.6 {
            return;
        }
        let eye = obs.eye();
        let aim = [target[0], target[1], target[2] + 8.0];
        let Some(throw) = grenade::solve(eye, aim, &offhand.facts) else {
            return;
        };
        if offhand.facts.fuse_ms > 0 && throw.flight_s * 1000.0 > offhand.facts.fuse_ms as f32 {
            return;
        }
        world.enter(QuerySubsystem::Tactical);
        if throw.arc_is_open(eye, &offhand.facts, world, obs.self_state.id) != Ok(true) {
            return;
        }
        let act = GrenadeAct {
            weapon: offhand.weapon,
            tactical,
            look_at: throw.look_at,
            phase: GrenadePhase::Aim,
            until: obs.tick.saturating_add(GRENADE_AIM_TICKS),
        };
        self.grenade = Some(act);
        self.continue_throw(obs, act);
    }

    fn continue_throw(&mut self, obs: &BotObservation, mut act: GrenadeAct) {
        if act.phase == GrenadePhase::Aim
            && (self.motor.aimed_at(obs, act.look_at, 3.0) || obs.tick >= act.until)
        {
            act.phase = GrenadePhase::Hold;
            act.until = obs.tick.saturating_add(GRENADE_HOLD_TICKS);
        } else if act.phase == GrenadePhase::Hold && obs.tick >= act.until {
            act.phase = GrenadePhase::Follow;
            act.until = obs.tick.saturating_add(GRENADE_FOLLOW_TICKS);
        } else if act.phase == GrenadePhase::Follow && obs.tick >= act.until {
            self.thrown[usize::from(act.tactical)] += 1;
            let wait = GRENADE_COOLDOWN_TICKS + (self.next_unit() * 240.0) as u32;
            self.grenade_ready_tick = obs.tick.saturating_add(wait);
            self.grenade = None;
            return;
        }
        self.grenade = Some(act);
        let held = act.phase == GrenadePhase::Hold;
        self.intent = BotIntent {
            look_at: Some(act.look_at),
            move_goal: Some(obs.self_state.origin),
            move_mode: MoveMode::Hold,
            path: PathOutcome::Clear,
            offhand: Some(act.weapon),
            frag: held && !act.tactical,
            smoke: held && act.tactical,
            ..BotIntent::default()
        };
    }

    /// Calls in the killstreak the script has handed over. Selecting its
    /// weapon is what uses it; the airstrikes then want a point on the map,
    /// and a marker or a sentry wants the trigger.
    fn call_streak(&mut self, obs: &BotObservation) {
        if self.grenade.is_some() || self.intent.use_button {
            return;
        }
        if obs.self_state.locating {
            self.intent.location_select = true;
            self.intent.fire = false;
            return;
        }
        let earned = obs
            .self_state
            .streak
            .filter(|streak| streak.kind != StreakKind::Crewed);
        let Some((weapon, kind, until)) = self.streak else {
            let quiet = self.memory.currently_seen().next().is_none() && self.alert.is_none();
            if let Some(streak) = earned
                && quiet
                && obs.tick >= self.streak_ready_tick
                && obs.self_state.weapon_action == WeaponAction::Ready
                && self.intent.weapon.is_none()
                && !self.intent.reload
            {
                self.streak = Some((
                    streak.weapon,
                    streak.kind,
                    obs.tick.saturating_add(STREAK_TICKS),
                ));
            }
            return;
        };
        let still_held = earned.is_some_and(|streak| streak.weapon == weapon);
        if !still_held || obs.tick >= until {
            self.streak = None;
            // Still in the slot when time ran out: it did not take. Leave it
            // a while before asking again.
            if still_held {
                self.streak_ready_tick = obs.tick.saturating_add(STREAK_RETRY_TICKS);
            }
            return;
        }
        self.intent.weapon = Some(weapon);
        self.intent.reload = false;
        self.intent.fire = false;
        self.intent.ads = false;
        self.intent.melee = false;
        self.intent.sprint = false;
        if kind == StreakKind::Thrown
            && obs.self_state.weapon == weapon
            && obs.self_state.weapon_action == WeaponAction::Ready
        {
            // Out in front and level: a marker lands a short throw ahead, a
            // sentry goes down where the bot is facing.
            let (sin, cos) = obs.self_state.viewangles[1].to_radians().sin_cos();
            let eye = obs.eye();
            self.intent.look_at = Some([eye[0] + cos * 300.0, eye[1] + sin * 300.0, eye[2]]);
            self.intent.move_mode = MoveMode::Hold;
            self.intent.place_held = true;
        }
    }

    /// Folds this observation into what the bot knows without seeing: the last
    /// place an enemy was placed, and the direction it was last hurt from.
    fn note_threats(&mut self, obs: &BotObservation) {
        // Nobody walks to where an enemy they watched die used to stand.
        if self.hotspot_of.is_some_and(|id| obs.gone.contains(&id)) {
            self.hotspot = None;
            self.hotspot_of = None;
        }
        let newest = self
            .memory
            .currently_seen()
            .chain(self.memory.last_seen())
            .max_by_key(|contact| contact.seen_tick)
            .copied();
        if let Some(contact) = newest
            && self
                .hotspot
                .is_none_or(|(_, tick)| contact.seen_tick >= tick)
        {
            self.hotspot = Some((contact.origin, contact.seen_tick));
            self.hotspot_of = Some(contact.id);
        }
        if self
            .hotspot
            .is_some_and(|(_, tick)| obs.tick.saturating_sub(tick) > HOTSPOT_TTL_TICKS)
        {
            self.hotspot = None;
            self.hotspot_of = None;
        }
        let in_sight = self.memory.currently_seen().next().is_some();
        let event = obs.self_state.damage_event;
        if self.damage_event.is_some_and(|old| old != event)
            && !in_sight
            && let Some(yaw) = obs.self_state.attacker_yaw
        {
            let (sin, cos) = yaw.to_radians().sin_cos();
            let eye = obs.eye();
            let point = [
                eye[0] + cos * ALERT_LOOK_IN,
                eye[1] + sin * ALERT_LOOK_IN,
                eye[2],
            ];
            self.alert = Some((point, obs.tick.saturating_add(ALERT_TICKS)));
            if self.hotspot.is_none() {
                let feet = obs.self_state.origin;
                self.hotspot = Some(([point[0], point[1], feet[2]], obs.tick));
                self.hotspot_of = None;
            }
        }
        self.damage_event = Some(event);
        if in_sight || self.alert.is_some_and(|(_, until)| obs.tick >= until) {
            self.alert = None;
        }
    }

    /// The remembered contact most worth walking to, with its utility. One
    /// that was seen and lost keeps the old flat value; one that was only
    /// heard is worth less the further off it is, so a bot on an objective is
    /// not pulled across the map by every shot.
    fn pick_investigation(&self, obs: &BotObservation) -> Option<(Contact, f32)> {
        let here = obs.self_state.origin;
        self.memory
            .last_seen()
            .filter(|c| obs.tick.saturating_sub(c.seen_tick) <= LAST_SEEN_FRESH_TICKS)
            .filter(|c| {
                self.investigate_block
                    .is_none_or(|(id, until)| id != c.id || obs.tick >= until)
            })
            .map(|c| {
                let score = if c.confidence >= 1.0 {
                    55.0
                } else {
                    (58.0 - dist2(here, c.origin).sqrt() / 90.0).clamp(22.0, 55.0)
                };
                (*c, score)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }

    fn next_unit(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// The route point a few steps on, at eye height. `None` when the route is
    /// too short for that to differ from the goal itself.
    fn look_ahead(&self, obs: &BotObservation) -> Option<[f32; 3]> {
        let last = self.committed.len().checked_sub(1)?;
        let step = self.committed[LOOK_AHEAD_STEPS.min(last)].position;
        let here = obs.self_state.origin;
        (dist_xy(here, step) > 48.0).then(|| [step[0], step[1], step[2] + (obs.eye()[2] - here[2])])
    }

    /// The end of the current strafe leg, starting a new one to the other side
    /// when the last is finished. A leg is only started onto ground the hull
    /// can reach and stand on; `None` leaves the bot where it is.
    fn strafe_goal(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        threat: [f32; 3],
    ) -> Option<[f32; 3]> {
        let here = obs.self_state.origin;
        if let Some((goal, until)) = self.strafe
            && obs.tick < until
            && dist_xy(here, goal) > 12.0
        {
            return Some(goal);
        }
        self.strafe = None;
        let to = [threat[0] - here[0], threat[1] - here[1]];
        let len = (to[0] * to[0] + to[1] * to[1]).sqrt();
        if len < 1.0 {
            return None;
        }
        // Mostly alternate, sometimes double back: a strict left-right rhythm
        // is as easy to hit as standing still.
        if self.next_unit() > 0.25 {
            self.strafe_side = -self.strafe_side;
        }
        world.enter(QuerySubsystem::Tactical);
        for side in [self.strafe_side, -self.strafe_side] {
            let goal = [
                here[0] - to[1] / len * side * STRAFE_STEP_IN,
                here[1] + to[0] / len * side * STRAFE_STEP_IN,
                here[2],
            ];
            if world.walk_hull(here, goal) != Ok(WalkSample::Clear) {
                continue;
            }
            let Ok(floor) = world.hull_trace(
                [goal[0], goal[1], goal[2] + 2.0],
                [goal[0], goal[1], goal[2] - nav::STEP_Z_IN],
            ) else {
                return None;
            };
            if floor.startsolid || floor.fraction >= 1.0 {
                continue;
            }
            self.strafe_side = side;
            let legs = STRAFE_LEG_TICKS + (self.next_unit() * 8.0) as u32;
            self.strafe = Some((floor.endpos, obs.tick.saturating_add(legs)));
            return Some(floor.endpos);
        }
        None
    }

    fn unstick(&mut self, obs: &BotObservation, world: &mut impl WorldQuery) -> Option<BotIntent> {
        let here = obs.self_state.origin;
        if self
            .progress_origin
            .is_none_or(|old| dist_xy(old, here) >= 16.0)
        {
            self.progress_origin = Some(here);
            self.progress_tick = obs.tick;
        }
        if let Some((goal, until)) = self.escape {
            if obs.tick < until && dist_xy(here, goal) > 8.0 {
                return Some(BotIntent {
                    look_at: Some(goal),
                    move_goal: Some(goal),
                    move_mode: MoveMode::Walk,
                    path: PathOutcome::Clear,
                    ..BotIntent::default()
                });
            }
            self.escape = None;
            self.committed.clear();
            self.roam = None;
        }
        let moving = matches!(
            self.intent.move_mode,
            MoveMode::Walk | MoveMode::Drop | MoveMode::BreakGlass
        ) || self.intent.path == PathOutcome::Blocked;
        if !moving || obs.tick.saturating_sub(self.progress_tick) < 40 {
            return None;
        }
        self.progress_tick = obs.tick;
        // Pmove can be blocked by other players even when world traces are clear.
        // Different clients yield in different directions before rejoining their route.
        let salt = obs
            .self_state
            .id
            .0
            .wrapping_mul(3)
            .wrapping_add(obs.tick / 40);
        for offset in 0..4 {
            let angle = ((salt + offset * 2) % 8) as f32 * core::f32::consts::FRAC_PI_4;
            let goal = [
                here[0] + angle.cos() * 64.0,
                here[1] + angle.sin() * 64.0,
                here[2],
            ];
            if world.walk_hull(here, goal) != Ok(WalkSample::Clear) {
                continue;
            }
            let Ok(floor) = world.hull_trace(
                [goal[0], goal[1], goal[2] + 2.0],
                [goal[0], goal[1], goal[2] - nav::STEP_Z_IN],
            ) else {
                continue;
            };
            if floor.startsolid || floor.fraction >= 1.0 {
                continue;
            }
            self.escape = Some((floor.endpos, obs.tick + 16));
            return Some(BotIntent {
                look_at: Some(floor.endpos),
                move_goal: Some(floor.endpos),
                move_mode: MoveMode::Walk,
                path: PathOutcome::Clear,
                // One hop as the escape starts: what held the bot is as often
                // a kerb or a sill as another player.
                jump: true,
                ..BotIntent::default()
            });
        }
        None
    }

    fn pick_task(&mut self, obs: &BotObservation, objective: Option<ModeObjective>) -> Task {
        let seen = self.memory.currently_seen().next().is_some();
        let investigation = self.pick_investigation(obs);
        self.investigate = investigation.map(|(contact, _)| contact);
        let mut scores = [
            10.0,
            investigation.map_or(-1.0, |(_, score)| score),
            if seen {
                80.0 + self.aggression * 10.0
            } else {
                -1.0
            },
            if self.wounded(obs) {
                if seen { 60.0 } else { 95.0 }
            } else {
                -1.0
            },
            -1.0,
        ];
        if seen && obs.self_state.ammo_clip == 0 {
            scores[2] = 30.0;
            scores[3] = 100.0;
        }
        if let Some(obj) = objective {
            let route_length = if self.route_goal == Some(obj.origin) && !self.committed.is_empty()
            {
                let mut from = obs.self_state.origin;
                let mut length = 0.0;
                for point in &self.committed {
                    length += dist2(from, point.position).sqrt();
                    from = point.position;
                }
                Some(length + dist2(from, obj.origin).sqrt())
            } else {
                None
            };
            scores[4] = objective_utility(obs, obj, route_length);
        }
        let kinds = [
            TaskKind::Hunt,
            TaskKind::Investigate,
            TaskKind::Fight,
            TaskKind::Recover,
            TaskKind::TouchObj,
        ];
        let mut best = 0;
        for i in 1..scores.len() {
            if scores[i] > scores[best] {
                best = i;
            }
        }
        let current = kinds.iter().position(|k| *k == self.task.kind).unwrap();
        if current != 0 && scores[current] >= 0.0 && scores[best] < scores[current] + SWITCH_MARGIN
        {
            best = current;
        }
        self.decision.scores = scores;
        Task {
            kind: kinds[best],
            reason: match kinds[best] {
                TaskKind::Hunt => {
                    if obs.objective.is_some() || !obs.objectives.is_empty() {
                        SwitchReason::PathFailed
                    } else {
                        SwitchReason::Spawned
                    }
                }
                TaskKind::Investigate => SwitchReason::LostSight,
                TaskKind::Fight => SwitchReason::SawEnemy,
                TaskKind::Recover => {
                    if self.wounded(obs) {
                        SwitchReason::LowHealth
                    } else {
                        SwitchReason::ClipEmpty
                    }
                }
                TaskKind::TouchObj => SwitchReason::ObjectiveReachable,
            },
        }
    }

    fn intent_for(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        objective: Option<ModeObjective>,
    ) -> BotIntent {
        match self.task.kind {
            TaskKind::Fight => self.fight(obs, world, nav, astar_budget),
            TaskKind::Investigate => self.seek(obs, world, nav, astar_budget, SeekKind::LastSeen),
            TaskKind::TouchObj => self.seek(
                obs,
                world,
                nav,
                astar_budget,
                SeekKind::Objective(objective),
            ),
            TaskKind::Recover => self.seek(obs, world, nav, astar_budget, SeekKind::BackOff),
            TaskKind::Hunt => self.hunt(obs, world, nav, astar_budget),
        }
    }

    fn hunt(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
    ) -> BotIntent {
        let Some(goal) = self.ensure_roam(obs, nav) else {
            return BotIntent::default();
        };
        let (move_mode, path, move_goal) =
            self.route(obs.self_state.origin, goal, world, nav, astar_budget);
        // Incomplete coverage is not a wall: pick another roam destination
        // instead of holding in front of the truncated region forever.
        if matches!(
            path,
            PathOutcome::Blocked | PathOutcome::Unreachable | PathOutcome::IncompleteGraph
        ) {
            // A hotspot with no route to it is not tried again on the next
            // sound from the same place: dating the roam goal ahead keeps
            // fresher word from redirecting the bot until the backoff is over.
            if self.roam_hot {
                self.roam_hot = false;
                self.roam_tick = Some(obs.tick.saturating_add(NO_ROUTE_BACKOFF_TICKS));
            }
            self.roam = None;
            self.committed.clear();
        }
        BotIntent {
            look_at: Some(
                self.look_ahead(obs)
                    .unwrap_or([goal[0], goal[1], obs.eye()[2]]),
            ),
            move_goal,
            desired_range: None,
            move_mode,
            path,
            ..BotIntent::default()
        }
    }

    fn ensure_roam(&mut self, obs: &BotObservation, nav: Option<&NavGraph>) -> Option<[f32; 3]> {
        // Word of an enemy outranks wandering: go to where one was last placed,
        // unless the bot is already heading for news about as fresh.
        if let Some((spot, tick)) = self.hotspot
            && self
                .roam_tick
                .is_none_or(|known| tick > known.saturating_add(HOTSPOT_REDIRECT_TICKS))
            && dist2(obs.self_state.origin, spot) > 96.0 * 96.0
        {
            self.hotspot = None;
            self.hotspot_of = None;
            self.roam_tick = Some(tick);
            self.roam_hot = true;
            self.roam = Some(spot);
            self.committed.clear();
            return Some(spot);
        }
        if let Some(goal) = self.roam
            && dist2(obs.self_state.origin, goal) > 64.0 * 64.0
        {
            return Some(goal);
        }
        if self.roam.is_some() {
            self.committed.clear();
        }
        self.roam_hot = false;
        self.roam_gen = self.roam_gen.saturating_add(1);
        let salt = obs.self_state.id.0 ^ self.roam_gen;
        let next = if let Some(graph) = nav.filter(|graph| !graph.is_empty()) {
            nav::roam_node(graph, obs.self_state.origin, salt)
        } else {
            let ang = (salt as f32) * 0.37;
            Some([
                obs.self_state.origin[0] + 160.0 * ang.cos(),
                obs.self_state.origin[1] + 160.0 * ang.sin(),
                obs.self_state.origin[2],
            ])
        };
        self.roam = next;
        next
    }

    fn fight(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
    ) -> BotIntent {
        let Some(target) = self.pick_fight_target(obs) else {
            return BotIntent::default();
        };
        if self.last_target != Some(target.id) {
            self.last_target = Some(target.id);
            // The level sets the delay; a bold bot shaves it, a careful one
            // takes longer to commit.
            let reaction = (self.skill.reaction_ticks as f32 * (1.25 - 0.5 * self.aggression))
                .round()
                .max(2.0) as u32;
            self.reaction_until = obs.tick.saturating_add(reaction);
        }
        let aim_at = target.aim_point();
        if self.wounded(obs) {
            return self.fight_break(obs, world, nav, astar_budget, target);
        }
        // Perception can see a head over cover while the chest is occluded.
        // Aim only at a point validated by the weapon trace, not sight alone.
        world.enter(QuerySubsystem::Tactical);
        let mut aim_at = aim_at;
        let mut shot = world.shot_ray(obs.eye(), aim_at, obs.self_state.id);
        if matches!(
            shot,
            Ok(SightSample::Blocked {
                obstacle: crate::query::ObstacleKind::Glass,
                ..
            })
        ) {
            return BotIntent {
                look_at: Some(aim_at),
                move_goal: Some(target.origin),
                move_mode: MoveMode::BreakGlass,
                path: PathOutcome::Clear,
                ..BotIntent::default()
            };
        }
        if definitely_obstructed(shot, target.id) {
            let head = if target.height > 1.0 {
                target.height + 4.0
            } else {
                64.0
            };
            for height in [head, 8.0] {
                let candidate = [
                    target.origin[0],
                    target.origin[1],
                    target.origin[2] + height,
                ];
                shot = world.shot_ray(obs.eye(), candidate, obs.self_state.id);
                if shot_allowed(shot, target.id) {
                    aim_at = candidate;
                    break;
                }
                if !definitely_obstructed(shot, target.id) {
                    break;
                }
            }
        }
        let clear = shot_allowed(shot, target.id);
        let range = dist2(obs.self_state.origin, target.origin).sqrt();
        let plan = ShotPlan::new(obs, target, range, clear);
        let fire = self.authorize_fire(
            obs.tick >= self.reaction_until
                && obs.self_state.ammo_clip != 0
                && fresh_contact(obs, target)
                && clear
                && plan.in_reach
                && !plan.melee,
            target,
        );
        let hold = self.hold_range(obs);
        // Inside the weapon's own range the fight is here. Further out, a clear
        // shot is still worth standing for unless the weapon is outranged or
        // the bot is the kind that pushes.
        let stand = range <= hold
            || (clear
                && range <= obs.self_state.weapon_class.engage_max()
                && self.aggression <= 0.6);
        let (mut move_mode, path, mut move_goal) =
            self.fight_position(obs, world, nav, astar_budget, aim_at, target, shot, stand);
        if matches!(
            path,
            PathOutcome::Unreachable | PathOutcome::Blocked | PathOutcome::IncompleteGraph
        ) && !shot_allowed(shot, target.id)
        {
            if let Some(obj) = choose_objective(
                obs,
                world,
                nav,
                astar_budget,
                &mut self.committed,
                &mut self.route_goal,
                &mut self.route_work,
            ) {
                self.task = Task {
                    kind: TaskKind::TouchObj,
                    reason: SwitchReason::PathFailed,
                };
                self.decision.objective = Some(obj);
                return self.seek(
                    obs,
                    world,
                    nav,
                    astar_budget,
                    SeekKind::Objective(Some(obj)),
                );
            }
            self.task = Task {
                kind: TaskKind::Hunt,
                reason: SwitchReason::PathFailed,
            };
            return self.hunt(obs, world, nav, astar_budget);
        }
        // Trading shots from one spot is how the first exchange is lost; keep
        // moving across the line of fire while the shot stays open. A scope is
        // the exception: it is only steady standing still.
        if move_mode == MoveMode::Hold
            && path == PathOutcome::Clear
            && clear
            && self.skill.strafes
            && !plan.melee
            && !plan.ads_required
        {
            if let Some(goal) = self.strafe_goal(obs, world, target.origin) {
                move_mode = MoveMode::Walk;
                move_goal = Some(goal);
            }
        } else {
            self.strafe = None;
        }
        plan.apply(BotIntent {
            look_at: Some(aim_at),
            move_goal,
            desired_range: Some(hold),
            move_mode,
            fire,
            path,
            ..BotIntent::default()
        })
    }

    fn pick_fight_target(&self, obs: &BotObservation) -> Option<Contact> {
        let hold = self.hold_range(obs);
        self.memory
            .currently_seen()
            .max_by(|a, b| {
                fight_score(obs, a, hold, self.last_target).total_cmp(&fight_score(
                    obs,
                    b,
                    hold,
                    self.last_target,
                ))
            })
            .copied()
    }

    fn wounded(&self, obs: &BotObservation) -> bool {
        obs.self_state.health > 0
            && (obs.self_state.health as f32) <= 20.0 + 20.0 * self.self_preserve
    }

    fn hold_range(&self, obs: &BotObservation) -> f32 {
        (obs.self_state.weapon_class.fight_hold() * (1.0 - 0.3 * self.aggression)).max(48.0)
    }

    fn fight_break(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        target: Contact,
    ) -> BotIntent {
        let aim_at = target.aim_point();
        let away = away_from(obs.self_state.origin, target.origin, 192.0);
        world.enter(QuerySubsystem::Tactical);
        let clear = shot_allowed(
            world.shot_ray(obs.eye(), aim_at, obs.self_state.id),
            target.id,
        );
        let range = dist2(obs.self_state.origin, target.origin).sqrt();
        // Backing off, the gun stays at the hip: raising the sights would slow
        // the retreat it is covering.
        let plan = ShotPlan {
            ads: false,
            ads_required: false,
            ..ShotPlan::new(obs, target, range, clear)
        };
        let fire = self.authorize_fire(
            self.aggression > self.self_preserve
                && obs.tick >= self.reaction_until
                && obs.self_state.ammo_clip != 0
                && fresh_contact(obs, target)
                && clear
                && plan.in_reach
                && !plan.melee,
            target,
        );
        let (move_mode, path, move_goal) =
            self.route(obs.self_state.origin, away, world, nav, astar_budget);
        plan.apply(BotIntent {
            look_at: Some(aim_at),
            move_goal,
            desired_range: None,
            move_mode,
            fire,
            path,
            ..BotIntent::default()
        })
    }

    /// Where to stand for this engagement. Both the reposition inside weapon
    /// range and the approach from outside it end at the same committed support:
    /// the bot moves to a place it can shoot from, never to the enemy's own
    /// coordinates, which change with every observation.
    #[allow(clippy::too_many_arguments)]
    fn fight_position(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        aim_at: [f32; 3],
        contact: Contact,
        shot: QueryResult<SightSample>,
        in_hold: bool,
    ) -> (MoveMode, PathOutcome, Option<[f32; 3]>) {
        let here = obs.self_state.origin;
        let target = contact.origin;
        let target_id = contact.id;
        world.enter(QuerySubsystem::Tactical);
        if shot.is_err() {
            return (MoveMode::Hold, PathOutcome::BudgetExhausted, Some(here));
        }
        // Already in range with a line of fire: this is the position.
        if in_hold && shot_allowed(shot, target_id) {
            return (MoveMode::Hold, PathOutcome::Clear, Some(here));
        }
        let hold = self.hold_range(obs);
        // The engagement, not the two exact coordinates: a threat that shifts a
        // little leaves the committed position standing. The bot's own position
        // is deliberately absent — walking to the position it chose must not
        // retire the choice.
        let key = FightScanKey {
            target: target_id,
            hold_in: hold as i32,
        };
        let mut scan = match self.fight_scan.take() {
            Some(scan) if scan.answers(key, target, obs.tick) => scan,
            _ => FightScan::begin(
                key,
                target,
                obs.tick,
                self.firing_candidates(obs, nav, target, hold),
            ),
        };
        // A committed position is walked to through the ordinary route executor,
        // which may take a detour; aim keeps following observations meanwhile.
        if let Some(pos) = scan.chosen {
            // Standing on the chosen support without a shot means it did not
            // deliver what it was chosen for; an unreachable one is no better.
            // Either way the next candidate is taken, never the same one again.
            let arrived = dist2(here, pos) <= WAYPOINT_IN * WAYPOINT_IN;
            let moved = (!arrived).then(|| self.route(here, pos, world, nav, astar_budget));
            let usable = moved.is_some_and(|moved| {
                !matches!(
                    moved.1,
                    PathOutcome::Unreachable | PathOutcome::IncompleteGraph
                )
            });
            if let Some(moved) = moved.filter(|_| usable) {
                scan.tick = obs.tick;
                self.fight_scan = Some(scan);
                return moved;
            }
            scan.commit_next(here);
            self.route_goal = None;
            self.committed.clear();
            if let Some(pos) = scan.chosen {
                let moved = self.route(here, pos, world, nav, astar_budget);
                scan.tick = obs.tick;
                self.fight_scan = Some(scan);
                return moved;
            }
        }
        let mut denied = false;
        let opened = scan.cursor;
        while (scan.cursor as usize) < scan.candidates.len() {
            let cand = scan.candidates[scan.cursor as usize];
            let eye = [
                cand[0],
                cand[1],
                cand[2]
                    + if obs.self_state.view_height > 1.0 {
                        obs.self_state.view_height
                    } else {
                        60.0
                    },
            ];
            let shot = world.shot_ray(eye, aim_at, obs.self_state.id);
            if shot.is_err() {
                denied = true;
                break;
            }
            if !shot_allowed(shot, target_id) {
                scan.cursor += 1;
                continue;
            }
            let threat_eye = [target[0], target[1], target[2] + 60.0];
            let chest = [cand[0], cand[1], cand[2] + 48.0];
            let return_fire = world.shot_ray(threat_eye, chest, target_id);
            if return_fire.is_err() {
                denied = true;
                break;
            }
            let exposed = return_fire_clear(return_fire, obs.self_state.id);
            // Travel and exposure in one comparable scale; reachability is left
            // to the route, so a position behind a corner is not thrown away.
            let score = dist2(here, cand).sqrt() + if exposed { 320.0 } else { 0.0 };
            scan.accepted.push((score, cand));
            scan.cursor += 1;
        }
        if !denied && scan.cursor as usize >= scan.candidates.len() {
            scan.commit_next(here);
        }
        let chosen = scan.chosen;
        let anchor = scan.anchor;
        // A visit that neither advanced the scan nor holds a position is not
        // progress, so it must not keep an exhausted scan alive: letting it age
        // out is what allows the engagement to be reconsidered.
        if scan.cursor != opened || chosen.is_some() {
            scan.tick = obs.tick;
        }
        self.fight_scan = Some(scan);
        let Some(pos) = chosen else {
            let path = if denied {
                PathOutcome::BudgetExhausted
            } else {
                PathOutcome::Blocked
            };
            // A visible enemy is not necessarily shootable. In range, let the
            // caller resume another goal when every firing position failed; out
            // of range, close on the engagement's anchor — a destination that
            // does not move with every observation and invalidate its own search.
            if in_hold || denied {
                return (MoveMode::Hold, path, Some(here));
            }
            return self.route(here, anchor, world, nav, astar_budget);
        };
        self.route(here, pos, world, nav, astar_budget)
    }

    /// The bounded candidate set a scan is frozen on. Supports come from the
    /// baked graph; the ring is only for a runtime that has no graph at all,
    /// where there is no support to choose from.
    fn firing_candidates(
        &self,
        obs: &BotObservation,
        nav: Option<&NavGraph>,
        target: [f32; 3],
        hold: f32,
    ) -> Vec<[f32; 3]> {
        let here = obs.self_state.origin;
        if let Some(graph) = nav.filter(|graph| !graph.is_empty()) {
            let supports = nav::firing_supports(graph, here, target, hold, FIGHT_CANDIDATES);
            if !supports.is_empty() {
                return supports;
            }
        }
        (0..FIGHT_RING)
            .map(|step| {
                let ang = (step as f32) * (core::f32::consts::TAU / FIGHT_RING as f32);
                [
                    target[0] + hold * ang.cos(),
                    target[1] + hold * ang.sin(),
                    here[2],
                ]
            })
            .collect()
    }

    fn seek(
        &mut self,
        obs: &BotObservation,
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
        kind: SeekKind,
    ) -> BotIntent {
        let (goal, use_button, look) = match kind {
            SeekKind::LastSeen => {
                let Some(last) = self
                    .investigate
                    .or_else(|| self.memory.last_seen().next().copied())
                else {
                    return BotIntent::default();
                };
                (last.origin, false, last.aim_point())
            }
            SeekKind::Objective(obj) => {
                let Some(obj) = obj else {
                    return BotIntent::default();
                };
                if obj.touching && obj.use_button {
                    return BotIntent {
                        look_at: Some(obj.origin),
                        move_goal: Some(obs.self_state.origin),
                        move_mode: MoveMode::Hold,
                        path: PathOutcome::Clear,
                        use_button: true,
                        ..BotIntent::default()
                    };
                }
                if obj.action == ObjectiveAction::Defend
                    || (obj.role == TeamRole::Cover && obj.active_user.is_some())
                {
                    // Scan separate sectors while guarding a site or an active teammate.
                    let angle = ((obs.self_state.id.0 + obs.tick / 80) % 8) as f32
                        * std::f32::consts::TAU
                        / 8.0;
                    let guard = [
                        obj.origin[0] + angle.cos() * 144.0,
                        obj.origin[1] + angle.sin() * 144.0,
                        obj.origin[2],
                    ];
                    if dist_xy(obs.self_state.origin, obj.origin) <= 220.0
                        && (obs.self_state.origin[2] - obj.origin[2]).abs()
                            <= nav::STEP_Z_IN * 2.0 + 8.0
                    {
                        return BotIntent {
                            look_at: Some([guard[0], guard[1], guard[2] + 60.0]),
                            path: PathOutcome::Clear,
                            ..BotIntent::default()
                        };
                    }
                }
                (
                    obj.origin,
                    false,
                    [obj.origin[0], obj.origin[1], obs.eye()[2]],
                )
            }
            SeekKind::BackOff => {
                let threat = self
                    .memory
                    .last_seen()
                    .next()
                    .or_else(|| self.memory.currently_seen().next())
                    .map(|c| c.origin)
                    .unwrap_or([
                        obs.self_state.origin[0] + 64.0,
                        obs.self_state.origin[1],
                        obs.self_state.origin[2],
                    ]);
                let away = away_from(obs.self_state.origin, threat, 192.0);
                (away, false, threat)
            }
        };
        let (move_mode, path, move_goal) =
            self.route(obs.self_state.origin, goal, world, nav, astar_budget);
        if kind == SeekKind::LastSeen
            && matches!(
                path,
                PathOutcome::Unreachable | PathOutcome::IncompleteGraph
            )
            && let Some(contact) = self.investigate
        {
            self.investigate_block =
                Some((contact.id, obs.tick.saturating_add(NO_ROUTE_BACKOFF_TICKS)));
        }
        BotIntent {
            look_at: Some(look),
            move_goal,
            desired_range: None,
            move_mode,
            use_button,
            crouch: matches!(kind, SeekKind::BackOff),
            path,
            ..BotIntent::default()
        }
    }

    fn route(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        world: &mut impl WorldQuery,
        nav: Option<&NavGraph>,
        astar_budget: &mut u32,
    ) -> (MoveMode, PathOutcome, Option<[f32; 3]>) {
        if let Some(graph) = nav.filter(|g| !g.is_empty()) {
            return self.route_nav(from, to, world, graph, astar_budget);
        }
        walk_step(from, to, world)
    }

    fn route_nav(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        world: &mut impl WorldQuery,
        graph: &NavGraph,
        astar_budget: &mut u32,
    ) -> (MoveMode, PathOutcome, Option<[f32; 3]>) {
        let need = self.committed.is_empty()
            || self
                .route_goal
                .is_none_or(|g| dist2(g, to) > WAYPOINT_IN * WAYPOINT_IN);
        if need {
            let objective = self.decision.objective.filter(|o| o.origin == to);
            match nav::find_route_resumable(
                world,
                graph,
                from,
                to,
                objective,
                astar_budget,
                &mut self.route_work,
            ) {
                Ok(path) => {
                    self.committed = path;
                    self.route_goal = Some(to);
                }
                Err(PathError::IncompleteGraph) => {
                    return (MoveMode::Hold, PathOutcome::IncompleteGraph, Some(to));
                }
                Err(PathError::BudgetExhausted { .. }) => {
                    return (MoveMode::Hold, PathOutcome::BudgetExhausted, Some(to));
                }
                Err(
                    PathError::GraphChanged
                    | PathError::Unreachable
                    | PathError::EmptyGraph
                    | PathError::NoStartSupport
                    | PathError::NoGoalSupport,
                ) => {
                    self.committed.clear();
                    return (MoveMode::Hold, PathOutcome::Unreachable, Some(to));
                }
            }
        }
        world.enter(QuerySubsystem::Execution);
        while let Some(step) = self.committed.first().copied() {
            let tolerance = if self.committed.len() == 1 && self.decision.objective.is_some() {
                4.0
            } else {
                WAYPOINT_IN
            };
            if dist2(from, step.position) > tolerance * tolerance {
                break;
            }
            if let Some(next) = self.committed.get(1) {
                // A special transition starts at its entry; do not cut its corner.
                if next.kind != nav::TraversalKind::Walk && dist2(from, next.entry) > 4.0 * 4.0 {
                    break;
                }
                if next.kind == nav::TraversalKind::Walk
                    && world.walk_hull(from, next.position) != Ok(WalkSample::Clear)
                    && dist_xy(from, step.position) > 4.0
                {
                    break;
                }
            }
            self.committed.remove(0);
        }
        let Some(step) = self.committed.first().copied() else {
            return (MoveMode::Hold, PathOutcome::Clear, Some(from));
        };
        let sample = if step.kind == nav::TraversalKind::Drop {
            nav::drop_clear(world, from, step.position)
        } else {
            world.walk_hull(from, step.position)
        };
        match sample {
            Ok(WalkSample::Clear) => (
                if step.kind == nav::TraversalKind::Drop {
                    MoveMode::Drop
                } else {
                    MoveMode::Walk
                },
                PathOutcome::Clear,
                Some(step.position),
            ),
            Ok(WalkSample::BreakGlass) => (
                MoveMode::BreakGlass,
                PathOutcome::Clear,
                Some(step.position),
            ),
            Ok(WalkSample::Blocked) => (MoveMode::Hold, PathOutcome::Blocked, Some(step.position)),
            Err(_) => (
                MoveMode::Hold,
                PathOutcome::BudgetExhausted,
                Some(step.position),
            ),
        }
    }
}

/// The engagement a firing position was chosen for: a different threat or a
/// different weapon band retires the choice. The bot's own movement towards the
/// position does not, and neither does a small move by the threat — that is the
/// `anchor` distance below, not a grid cell, so no boundary makes one step
/// count as a new engagement.
/// One grenade, from deciding to throw it to the arm coming back down.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GrenadeAct {
    weapon: u16,
    tactical: bool,
    look_at: [f32; 3],
    phase: GrenadePhase,
    until: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GrenadePhase {
    /// Turning to the throwing angle.
    Aim,
    /// Button down: the pin is out.
    Hold,
    /// Button up: the throw is on its way out of the hand.
    Follow,
}

/// How the weapon in hand is worked against one target at one range: sights,
/// trigger discipline and how tight the aim has to be before a shot goes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShotPlan {
    ads: bool,
    ads_required: bool,
    melee: bool,
    tap_fire: bool,
    burst: bool,
    fire_cone_deg: f32,
    /// The target is close enough for this weapon to be worth firing at all.
    /// A shotgun emptied at a rifle's range is ammunition thrown away.
    in_reach: bool,
}

impl ShotPlan {
    fn new(obs: &BotObservation, target: Contact, range: f32, clear: bool) -> Self {
        let class = obs.self_state.weapon_class;
        let facts = obs.held().map(|slot| slot.facts);
        let tap_fire = facts.is_some_and(|facts| facts.fire_single);
        let melee = clear && range <= MELEE_RANGE_IN;
        // A weapon the content has no row for is assumed to have sights. They
        // only come up for a shot that is open: walking to a firing position
        // with them raised is walking there at half speed.
        let ads = facts.is_none_or(|facts| facts.can_ads)
            && clear
            && range >= class.ads_min()
            && fresh_contact(obs, target)
            && !melee;
        Self {
            ads,
            ads_required: ads && class == crate::observation::WeaponClass::Sniper,
            melee,
            tap_fire,
            burst: !tap_fire && range > BURST_RANGE_IN,
            fire_cone_deg: fire_cone(range),
            in_reach: range <= class.engage_max() * 1.25,
        }
    }

    fn apply(self, intent: BotIntent) -> BotIntent {
        BotIntent {
            ads: self.ads,
            ads_required: self.ads_required,
            melee: self.melee,
            tap_fire: self.tap_fire,
            burst: self.burst,
            fire_cone_deg: self.fire_cone_deg,
            ..intent
        }
    }
}

/// Half-angle the view must be inside before a shot. About one and a half
/// body-widths at the target's range, plus a degree: wide enough up close that
/// turning never holds fire, narrow enough at range that rounds are not thrown
/// past a target the view has not reached yet.
fn fire_cone(range: f32) -> f32 {
    ((20.0 / range.max(1.0)).atan().to_degrees() * 1.5 + 1.0).clamp(1.5, DEFAULT_FIRE_CONE_DEG)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FightScanKey {
    target: ClientId,
    hold_in: i32,
}

/// One bounded evaluation of firing positions and the position it committed to.
/// The candidate list is frozen when the scan begins, so a denied slice resumes
/// at the same candidate instead of re-deriving the set.
#[derive(Clone, Debug, PartialEq)]
struct FightScan {
    key: FightScanKey,
    /// Where the threat was when this set was derived.
    anchor: [f32; 3],
    tick: u32,
    cursor: u32,
    candidates: Vec<[f32; 3]>,
    /// Candidates that passed their shot and exposure checks, with their score.
    /// A position that is taken or rejected leaves this list, so the next choice
    /// costs no queries and the same one is never re-picked.
    accepted: Vec<(f32, [f32; 3])>,
    chosen: Option<[f32; 3]>,
}

impl FightScan {
    fn begin(key: FightScanKey, anchor: [f32; 3], tick: u32, candidates: Vec<[f32; 3]>) -> Self {
        Self {
            key,
            anchor,
            tick,
            cursor: 0,
            candidates,
            accepted: Vec::new(),
            chosen: None,
        }
    }

    /// Whether this scan still answers the engagement in front of the bot.
    fn answers(&self, key: FightScanKey, at: [f32; 3], tick: u32) -> bool {
        self.key == key
            && dist2(self.anchor, at) <= FIGHT_COMMIT_MOVE * FIGHT_COMMIT_MOVE
            && !self.expired(tick)
    }

    /// Commits to the cheapest remaining accepted position the bot is not
    /// already standing on — one that close has just failed its own shot test
    /// from here, and the motor could not travel to it anyway.
    fn commit_next(&mut self, here: [f32; 3]) -> Option<[f32; 3]> {
        self.chosen = None;
        loop {
            let at = self
                .accepted
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.0.total_cmp(&b.1.0))
                .map(|(index, _)| index)?;
            let pos = self.accepted.remove(at).1;
            if dist2(here, pos) > WAYPOINT_IN * WAYPOINT_IN {
                self.chosen = Some(pos);
                return self.chosen;
            }
        }
    }

    /// An unfinished scan is short-lived; a commitment lives as long as the
    /// engagement it was made for, and no longer.
    fn expired(&self, tick: u32) -> bool {
        let age = tick.saturating_sub(self.tick);
        if self.chosen.is_some() {
            age > FIGHT_COMMIT_MAX_AGE_TICKS
        } else {
            age > FIGHT_SCAN_MAX_AGE_TICKS
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum SeekKind {
    LastSeen,
    Objective(Option<ModeObjective>),
    BackOff,
}

fn objective_utility(obs: &BotObservation, obj: ModeObjective, route_length: Option<f32>) -> f32 {
    let travel_ms = route_length.unwrap_or_else(|| dist2(obs.self_state.origin, obj.origin).sqrt())
        / 180.0
        * 1000.0;
    let cost = (travel_ms / 1000.0).min(25.0);
    let interaction_left = obj.interaction_ms as f32 * (1.0 - obj.progress.clamp(0.0, 1.0));
    if obj.action == ObjectiveAction::Defuse
        && obj.role == TeamRole::Actor
        && obj.remaining_ms.is_some_and(|ms| {
            (ms as f32) < interaction_left + if obj.touching { 0.0 } else { travel_ms }
        })
    {
        return -1.0;
    }
    let mut value = match obj.action {
        ObjectiveAction::Capture => 40.0 - cost,
        ObjectiveAction::Plant => 70.0 - cost,
        ObjectiveAction::Defend => 55.0 - cost,
        ObjectiveAction::Defuse => {
            let needed =
                travel_ms + obj.interaction_ms as f32 * (1.0 - obj.progress.clamp(0.0, 1.0));
            let slack = obj.remaining_ms.unwrap_or(u32::MAX) as f32 - needed;
            if slack < 0.0 {
                20.0
            } else {
                110.0 + (30.0 - slack / 1000.0).clamp(0.0, 30.0)
            }
        }
    };
    if obj.role == TeamRole::Cover {
        value = value.min(62.0);
    }
    if obj.touching && obj.use_button {
        value = 200.0;
    }
    value
}

fn choose_objective(
    obs: &BotObservation,
    world: &mut impl WorldQuery,
    nav: Option<&NavGraph>,
    astar_budget: &mut u32,
    committed: &mut Vec<nav::RouteStep>,
    route_goal: &mut Option<[f32; 3]>,
    route_work: &mut nav::RouteWork,
) -> Option<ModeObjective> {
    let mut candidates = obs.objectives.clone();
    if candidates.is_empty()
        && let Some(obj) = obs.objective
    {
        candidates.push(obj);
    }
    candidates.sort_by(|a, b| {
        let score = |o: &ModeObjective| {
            objective_utility(obs, *o, None)
                + if *route_goal == Some(o.origin) {
                    6.0
                } else {
                    0.0
                }
        };
        score(b).total_cmp(&score(a)).then_with(|| a.id.cmp(&b.id))
    });
    let Some(graph) = nav.filter(|graph| !graph.is_empty()) else {
        return candidates.first().copied();
    };
    let mut deferred: Option<ModeObjective> = None;
    for obj in candidates {
        if *route_goal == Some(obj.origin)
            && (obj.touching || path_ends_in_use_volume(world, committed, obj))
        {
            return Some(obj);
        }
        match nav::find_route_resumable(
            world,
            graph,
            obs.self_state.origin,
            obj.origin,
            Some(obj),
            astar_budget,
            route_work,
        ) {
            Ok(path) => {
                if path_ends_in_use_volume(world, &path, obj) {
                    *committed = path;
                    *route_goal = Some(obj.origin);
                    return Some(obj);
                }
            }
            // Budget-deferred work stays pending: stop here and keep the slot.
            Err(PathError::BudgetExhausted { .. }) => {
                deferred = Some(obj);
                break;
            }
            // Coverage that cannot answer this request is not proof of global
            // unreachability, and waiting does not extend the published graph.
            // Record it and let a lower-scoring, routable objective be tried.
            Err(
                PathError::GraphChanged
                | PathError::IncompleteGraph
                | PathError::Unreachable
                | PathError::EmptyGraph
                | PathError::NoStartSupport
                | PathError::NoGoalSupport,
            ) => {}
        }
    }
    deferred
}

/// A positive observation of this contact, recent enough to shoot at. A contact
/// held through unsensed ticks still aims and moves; it does not fire.
fn fresh_contact(obs: &BotObservation, target: Contact) -> bool {
    target.source == KnowledgeSource::CurrentlySeen
        && obs.tick.saturating_sub(target.seen_tick) <= FIRE_FRESH_TICKS
}

fn shot_allowed(sample: QueryResult<SightSample>, target: ClientId) -> bool {
    match sample {
        Ok(SightSample::Clear) => true,
        Ok(SightSample::HitPlayer { client, .. }) => client == target,
        Ok(SightSample::Unknown | SightSample::Blocked { .. }) | Err(_) => false,
    }
}

/// A trace that ran and found something in the way. A denied or unclassified
/// trace says nothing about the geometry, so it is not an obstruction.
fn definitely_obstructed(sample: QueryResult<SightSample>, target: ClientId) -> bool {
    matches!(
        sample,
        Ok(SightSample::Blocked { .. }) | Ok(SightSample::HitPlayer { .. })
    ) && !shot_allowed(sample, target)
}

fn return_fire_clear(sample: QueryResult<SightSample>, bot: ClientId) -> bool {
    match sample {
        Ok(SightSample::Clear) => true,
        Ok(SightSample::HitPlayer { client, .. }) => client == bot,
        Ok(SightSample::Unknown | SightSample::Blocked { .. }) | Err(_) => false,
    }
}

fn fight_score(obs: &BotObservation, contact: &Contact, hold: f32, last: Option<ClientId>) -> f32 {
    let range = dist2(obs.self_state.origin, contact.origin).sqrt();
    let dist = 1.0 - (range - hold).abs() / (hold + 80.0);
    let stick = if last == Some(contact.id) { 0.4 } else { 0.0 };
    dist + stick
}

fn away_from(here: [f32; 3], threat: [f32; 3], dist: f32) -> [f32; 3] {
    let dx = here[0] - threat[0];
    let dy = here[1] - threat[1];
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    [
        here[0] + dx / len * dist,
        here[1] + dy / len * dist,
        here[2],
    ]
}

fn path_ends_in_use_volume(
    world: &impl WorldQuery,
    path: &[nav::RouteStep],
    obj: ModeObjective,
) -> bool {
    path.last()
        .is_some_and(|end| world.objective_contains(obj, end.position))
}

fn walk_step(
    from: [f32; 3],
    to: [f32; 3],
    world: &mut impl WorldQuery,
) -> (MoveMode, PathOutcome, Option<[f32; 3]>) {
    let d = [to[0] - from[0], to[1] - from[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if len < 8.0 {
        return (MoveMode::Hold, PathOutcome::Clear, Some(to));
    }
    let scale = (STEP_IN / len).min(1.0);
    let step = [from[0] + d[0] * scale, from[1] + d[1] * scale, from[2]];
    match world.walk_hull(from, step) {
        Ok(WalkSample::Clear) => (MoveMode::Walk, PathOutcome::Clear, Some(to)),
        Ok(WalkSample::BreakGlass) => (MoveMode::BreakGlass, PathOutcome::Clear, Some(step)),
        Ok(WalkSample::Blocked) if len > STEP_IN => {
            (MoveMode::Hold, PathOutcome::Unreachable, Some(to))
        }
        Ok(WalkSample::Blocked) => (MoveMode::Hold, PathOutcome::Blocked, Some(to)),
        Err(_) => (MoveMode::Hold, PathOutcome::BudgetExhausted, Some(to)),
    }
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

fn dist_xy(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}
