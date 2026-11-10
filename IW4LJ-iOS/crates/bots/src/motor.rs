use math_iw4::{angle_normalize_360, angle_subtract, yaw_vectors_2d};
use movement_iw4::ANGLE2SHORT;
use playerstate_iw4::{UserCmd, buttons};

use crate::intent::{BotIntent, DEFAULT_FIRE_CONE_DEG, MotorReport, MoveMode, PathOutcome};
use crate::observation::BotObservation;
use crate::skill::BotSkill;

const MAX_YAW_DEG_PER_MS: f32 = 0.36;
const MAX_PITCH_DEG_PER_MS: f32 = 0.24;
const ARRIVE_IN: f32 = 24.0;
const PITCH_LIMIT: f32 = 70.0;
/// A look target that jumps further than this in one step is a new target, and
/// the aim has to settle on it again.
const REACQUIRE_IN: f32 = 96.0;
/// How often the settled aim picks a new point to drift toward.
const WANDER_PERIOD_MS: f32 = 240.0;
/// Automatic fire at range: this many driven steps on, then this many off.
const BURST_ON: u32 = 3;
const BURST_OFF: u32 = 2;
/// How far the sights must be up before a weapon that needs them may fire.
const ADS_READY: f32 = 0.85;

#[derive(Clone, Debug)]
pub struct Motor {
    yaw: f32,
    pitch: f32,
    skill: BotSkill,
    rng: u64,
    /// The aim's current offset from the true line, in units of the error
    /// amplitude, and the point it is drifting toward.
    wander: [f32; 2],
    wander_goal: [f32; 2],
    wander_ms: f32,
    /// Milliseconds spent on the current look target.
    settle_ms: f32,
    burst_step: u32,
    last_look: Option<[f32; 3]>,
    armed: Option<sim::LifeSequence>,
}

impl Motor {
    pub fn with_skill(seed: u64, skill: BotSkill) -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            skill,
            // xorshift has no zero state.
            rng: seed | 1,
            wander: [0.0; 2],
            wander_goal: [0.0; 2],
            wander_ms: WANDER_PERIOD_MS,
            settle_ms: 0.0,
            burst_step: 0,
            last_look: None,
            armed: None,
        }
    }

    pub fn set_skill(&mut self, skill: BotSkill) {
        self.skill = skill;
    }

    pub fn report(&self, obs: &BotObservation, intent: &BotIntent, cmd: &UserCmd) -> MotorReport {
        if matches!(intent.move_mode, MoveMode::Mantle | MoveMode::Ladder) {
            return MotorReport::Unsupported;
        }
        if intent.path == PathOutcome::BudgetExhausted || intent.path == PathOutcome::ProgressLost {
            return MotorReport::Executing;
        }
        if intent.path == PathOutcome::Blocked || intent.path == PathOutcome::Unreachable {
            return MotorReport::Blocked;
        }
        if intent.move_mode == MoveMode::Drop {
            let Some(goal) = intent.move_goal else {
                return MotorReport::Executing;
            };
            if obs.self_state.origin[2] - goal[2] > crate::nav::STEP_Z_IN {
                return MotorReport::Executing;
            }
            if dist2(obs.self_state.origin, goal) <= ARRIVE_IN * ARRIVE_IN {
                return MotorReport::Arrived;
            }
            return MotorReport::Executing;
        }
        if intent.move_mode == MoveMode::Hold || (cmd.forwardmove == 0 && cmd.rightmove == 0) {
            if let Some(goal) = intent.move_goal
                && dist2(obs.self_state.origin, goal) <= ARRIVE_IN * ARRIVE_IN
            {
                return MotorReport::Arrived;
            }
            return MotorReport::Executing;
        }
        MotorReport::Executing
    }

    /// `dt_ms` is the time since the previous call, which is the think
    /// interval and not necessarily one authority tick.
    pub fn drive(&mut self, obs: &BotObservation, intent: &BotIntent, dt_ms: i32) -> UserCmd {
        if self.armed != Some(obs.self_state.life_sequence) {
            self.yaw = obs.self_state.viewangles[1];
            self.pitch = angle_subtract(obs.self_state.viewangles[0], 0.0);
            self.armed = Some(obs.self_state.life_sequence);
            self.last_look = None;
            self.burst_step = 0;
        }
        let dt = dt_ms.max(1) as f32;
        let mut error_deg = self.skill.aim_error_deg;
        if let Some(at) = intent.look_at {
            if self
                .last_look
                .is_none_or(|old| dist2(old, at) > REACQUIRE_IN * REACQUIRE_IN)
            {
                self.settle_ms = 0.0;
            } else {
                self.settle_ms += dt;
            }
            self.last_look = Some(at);
            self.drift(dt);
            error_deg += self.skill.acquire_error_deg
                * (-self.settle_ms / self.skill.settle_ms.max(1.0)).exp();
            let desire = look_angles(obs.eye(), at);
            // The fraction of the remaining error closed per 100 ms. Scaling it
            // by the step keeps the feel the same at any think rate.
            let gain = (0.45 + 0.2 * self.skill.turn_scale).min(0.9);
            let gain = 1.0 - (1.0 - gain).powf(dt / 100.0);
            let yaw_err = angle_subtract(desire[1] + self.wander[0] * error_deg, self.yaw);
            self.yaw = angle_normalize_360(
                self.yaw
                    + slew(
                        yaw_err,
                        MAX_YAW_DEG_PER_MS * self.skill.turn_scale,
                        gain,
                        dt,
                    ),
            );
            // A standing target is taller than it is wide, so the vertical
            // error is held to half the horizontal one.
            let pitch_goal =
                (desire[0] + self.wander[1] * error_deg * 0.5).clamp(-PITCH_LIMIT, PITCH_LIMIT);
            self.pitch = (self.pitch
                + slew(
                    pitch_goal - self.pitch,
                    MAX_PITCH_DEG_PER_MS * self.skill.turn_scale,
                    gain,
                    dt,
                ))
            .clamp(-PITCH_LIMIT, PITCH_LIMIT);
        } else {
            self.last_look = None;
        }
        let unsupported = matches!(intent.move_mode, MoveMode::Mantle | MoveMode::Ladder);
        let walking = matches!(intent.move_mode, MoveMode::Walk | MoveMode::Drop)
            && matches!(intent.path, PathOutcome::Clear | PathOutcome::ProgressLost);
        let (mut forwardmove, mut rightmove) = (0i8, 0i8);
        if !unsupported
            && walking
            && let Some(goal) = intent.move_goal
        {
            let wish = [
                goal[0] - obs.self_state.origin[0],
                goal[1] - obs.self_state.origin[1],
            ];
            (forwardmove, rightmove) = wish_to_move(self.yaw, wish);
        }
        // A requested weapon has to survive the whole change: the simulator
        // settles a dropping hand back to ready when the command asks for the
        // weapon it is already lowering.
        let weapon = intent.weapon.unwrap_or(obs.self_state.weapon);
        let delta = obs.self_state.delta_angles;
        let mut cmd = UserCmd {
            server_time: obs.time_ms,
            angles: [
                ((self.pitch - delta[0]) * ANGLE2SHORT) as i32,
                ((self.yaw - delta[1]) * ANGLE2SHORT) as i32,
                0,
            ],
            forwardmove,
            rightmove,
            weapon,
            weapon_mapped: weapon,
            ..UserCmd::default()
        };
        if self.pull_trigger(obs, intent)
            || (intent.place_held && self.on_target(obs, intent, DEFAULT_FIRE_CONE_DEG))
        {
            cmd.buttons |= buttons::ATTACK;
        }
        if intent.melee && self.on_target(obs, intent, DEFAULT_FIRE_CONE_DEG) {
            cmd.buttons |= buttons::MELEE_CHARGE;
        }
        if intent.ads {
            cmd.buttons |= buttons::ADS;
        }
        if intent.use_button {
            cmd.buttons |= buttons::USE;
        }
        if intent.reload {
            cmd.buttons |= buttons::RELOAD;
        }
        if intent.jump {
            cmd.buttons |= buttons::JUMP;
        }
        if let Some(offhand) = intent.offhand {
            cmd.off_hand_index = offhand;
        }
        if intent.frag {
            cmd.buttons |= buttons::FRAG;
        }
        if intent.smoke {
            cmd.buttons |= buttons::SMOKE;
        }
        // The selected point is left at its zero value, which the map reads
        // as its own centre.
        if intent.location_select {
            cmd.buttons |= buttons::LOCATION_SELECT;
        }
        if intent.sprint
            && walking
            && intent.move_mode == MoveMode::Walk
            && forwardmove > 110
            && rightmove.abs() < 45
            && !intent.fire
            && !intent.ads
            && !intent.use_button
            && !intent.reload
            && !intent.crouch
        {
            cmd.buttons |= buttons::SPRINT;
        }
        if intent.crouch {
            cmd.buttons |= buttons::CROUCH;
        }
        cmd
    }

    /// Whether this step presses the trigger. Being allowed to fire is the
    /// controller's decision; whether the gun is pointing well enough, the
    /// sights are up and the burst is in its firing half is decided here.
    fn pull_trigger(&mut self, obs: &BotObservation, intent: &BotIntent) -> bool {
        if !intent.fire {
            self.burst_step = 0;
            return false;
        }
        // The cone never closes tighter than the bot can hold the gun.
        let cone = intent
            .fire_cone_deg
            .max(self.skill.aim_error_deg * 1.25 + 0.5);
        if !self.on_target(obs, intent, cone) {
            return false;
        }
        if intent.ads_required && obs.self_state.ads < ADS_READY {
            return false;
        }
        if intent.burst && !intent.tap_fire {
            let firing = self.burst_step < BURST_ON;
            self.burst_step = (self.burst_step + 1) % (BURST_ON + BURST_OFF);
            return firing;
        }
        true
    }

    fn on_target(&self, obs: &BotObservation, intent: &BotIntent, cone_deg: f32) -> bool {
        intent
            .look_at
            .is_some_and(|at| self.aimed_at(obs, at, cone_deg))
    }

    /// Whether the view has come within `cone_deg` of a point.
    pub fn aimed_at(&self, obs: &BotObservation, at: [f32; 3], cone_deg: f32) -> bool {
        let desire = look_angles(obs.eye(), at);
        angle_subtract(desire[1], self.yaw).abs() <= cone_deg
            && (desire[0] - self.pitch).abs() <= cone_deg
    }

    /// Moves the aim offset toward its current drift point and picks a new
    /// point every period, so shots scatter around a target instead of sitting
    /// at one fixed offset from it.
    fn drift(&mut self, dt: f32) {
        self.wander_ms += dt;
        if self.wander_ms >= WANDER_PERIOD_MS {
            self.wander_ms = 0.0;
            self.wander_goal = [self.next_signed(), self.next_signed()];
        }
        let step = (dt / WANDER_PERIOD_MS).min(1.0);
        for axis in 0..2 {
            self.wander[axis] += (self.wander_goal[axis] - self.wander[axis]) * step;
        }
    }

    /// Uniform in [-1, 1).
    fn next_signed(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        ((self.rng >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }
}

fn look_angles(from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
    let mut angles = math_iw4::vect_to_angles([to[0] - from[0], to[1] - from[1], to[2] - from[2]]);
    // vect_to_angles wraps upward pitch into [0, 360); the motor clamps
    // and compares pitch in the signed range.
    angles[0] = angle_subtract(angles[0], 0.0);
    angles
}

/// One step toward closing `err`: a fraction of what is left, capped at the
/// turn rate. Converging by a fraction is what makes the view settle onto a
/// target rather than snap to it.
fn slew(err: f32, max_rate: f32, gain: f32, dt: f32) -> f32 {
    (err * gain).clamp(-max_rate * dt, max_rate * dt)
}

fn wish_to_move(yaw_deg: f32, wish: [f32; 2]) -> (i8, i8) {
    let len = (wish[0] * wish[0] + wish[1] * wish[1]).sqrt();
    if len < 1.0 {
        return (0, 0);
    }
    let (forward, right) = yaw_vectors_2d(yaw_deg);
    let f = (wish[0] * forward[0] + wish[1] * forward[1]) / len;
    let r = (wish[0] * right[0] + wish[1] * right[1]) / len;
    (
        (f * 127.0).round().clamp(-127.0, 127.0) as i8,
        (r * 127.0).round().clamp(-127.0, 127.0) as i8,
    )
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}
