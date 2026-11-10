//! Behaviour checks for the controller, the motor and what they are fed. The
//! world here is an open floor with no collision unless a test asks for a
//! wall, so every expectation is about a decision, not about a map.

use movement_iw4::ANGLE2SHORT;
use playerstate_iw4::{UserCmd, buttons};
use sim::{ClientId, ClientLifecycle, LifeSequence};

use crate::controller::HostController;
use crate::intent::MoveMode;
use crate::memory::Memory;
use crate::observation::{
    BotEvent, BotObservation, Contact, KnowledgeSource, ModeObjective, Offhand, SelfState, Stance,
    Streak, StreakKind, ThrowFacts, ThrowKind, WeaponAction, WeaponClass, WeaponFacts, WeaponSlot,
};
use crate::query::{HullTrace, ObstacleKind, QueryResult, SightSample, WorldQuery};
use crate::roster::default_class_index;
use crate::sensor::{audible, bearing_to_attacker};
use crate::skill::SkillLevel;
use crate::task::TaskKind;
use crate::weapon::WeaponSkill;

const RIFLE: u16 = 7;
/// Low aggression, so the bot holds a clear shot instead of pushing.
const CALM_SEED: u64 = 0x40;
const THINK_MS: i32 = 100;

struct Field {
    walled: bool,
    class: WeaponClass,
    facts: WeaponFacts,
}

impl Field {
    fn open(class: WeaponClass) -> Self {
        Self {
            walled: false,
            class,
            facts: WeaponFacts {
                selectable: true,
                clip_size: 30,
                reload_time_ms: 2000,
                reload_empty_time_ms: 2500,
                can_ads: true,
                ..WeaponFacts::default()
            },
        }
    }

    fn ray(&self) -> QueryResult<SightSample> {
        Ok(if self.walled {
            SightSample::Blocked {
                fraction: 0.5,
                obstacle: ObstacleKind::World,
            }
        } else {
            SightSample::Clear
        })
    }
}

impl WorldQuery for Field {
    fn sight_ray(&mut self, _: [f32; 3], _: [f32; 3], _: ClientId) -> QueryResult<SightSample> {
        self.ray()
    }

    fn shot_ray(&mut self, _: [f32; 3], _: [f32; 3], _: ClientId) -> QueryResult<SightSample> {
        self.ray()
    }

    /// A flat floor at z = 0 and nothing else to hit.
    fn hull_trace(&mut self, start: [f32; 3], end: [f32; 3]) -> QueryResult<HullTrace> {
        let fraction = if start[2] > 0.0 && end[2] < 0.0 {
            start[2] / (start[2] - end[2])
        } else {
            1.0
        };
        Ok(HullTrace {
            fraction,
            normal: [0.0, 0.0, 1.0],
            endpos: std::array::from_fn(|i| start[i] + (end[i] - start[i]) * fraction),
            startsolid: false,
        })
    }

    fn weapon_class(&self, _: u16) -> WeaponClass {
        self.class
    }

    fn weapon_facts(&self, _: u16) -> Option<WeaponFacts> {
        Some(self.facts)
    }
}

fn me(id: u32, field: &Field) -> SelfState {
    SelfState {
        life_sequence: LifeSequence(1),
        id: ClientId(id),
        lifecycle: ClientLifecycle::Alive,
        origin: [0.0; 3],
        viewangles: [0.0; 3],
        delta_angles: [0.0; 3],
        view_height: 60.0,
        stance: Stance::Stand,
        health: 100,
        weapon: RIFLE,
        weapon_class: field.class,
        weapon_action: WeaponAction::Ready,
        ammo_clip: 30,
        ammo_stock: 90,
        team: 1,
        velocity: [0.0; 3],
        ads: 0.0,
        damage_event: 0,
        attacker_yaw: None,
        radar: false,
        lethal: None,
        tactical: None,
        streak: None,
        locating: false,
    }
}

fn observe(tick: u32, self_state: SelfState, field: &Field) -> BotObservation {
    BotObservation {
        tick,
        time_ms: tick as i32 * 50,
        inventory: vec![WeaponSlot {
            weapon: RIFLE,
            class: field.class,
            facts: field.facts,
            clip: self_state.ammo_clip,
            stock: self_state.ammo_stock,
        }],
        self_state,
        seen: Vec::new(),
        heard: Vec::new(),
        gone: Vec::new(),
        unsensed: Vec::new(),
        events: Vec::new(),
        objective: None,
        objectives: Vec::new(),
    }
}

fn enemy(id: u32, origin: [f32; 3], tick: u32, source: KnowledgeSource) -> Contact {
    Contact {
        id: ClientId(id),
        origin,
        source,
        seen_tick: tick,
        confidence: if source == KnowledgeSource::CurrentlySeen {
            1.0
        } else {
            0.5
        },
        height: 60.0,
    }
}

fn yaw_of(cmd: &UserCmd) -> f32 {
    math_iw4::angle_normalize_360(cmd.angles[1] as f32 / ANGLE2SHORT)
}

/// Drives one bot against one enemy that stays in sight, on the odd ticks the
/// host thinks on, and returns every command it produced.
fn fight(
    brain: &mut HostController,
    field: &mut Field,
    at: [f32; 3],
    thinks: u32,
    ads: f32,
) -> Vec<UserCmd> {
    (0..thinks)
        .map(|step| {
            let tick = 1 + step * 2;
            let mut obs = observe(
                tick,
                SelfState {
                    ads,
                    ..me(1, field)
                },
                field,
            );
            obs.seen
                .push(enemy(9, at, tick, KnowledgeSource::CurrentlySeen));
            brain.drive(&obs, &mut *field, THINK_MS)
        })
        .collect()
}

#[test]
fn a_visible_enemy_is_shot_after_the_reaction_delay_with_the_sights_up() {
    let mut field = Field::open(WeaponClass::Assault);
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
    let cmds = fight(&mut brain, &mut field, [600.0, 0.0, 0.0], 20, 1.0);
    assert_eq!(brain.task().kind, TaskKind::Fight);
    assert_eq!(
        cmds[0].buttons & buttons::ATTACK,
        0,
        "nobody fires on the tick they first see someone"
    );
    let first = cmds
        .iter()
        .position(|cmd| cmd.buttons & buttons::ATTACK != 0)
        .expect("a bot with a clear shot fires within two seconds");
    assert!(
        cmds[first].buttons & buttons::ADS != 0,
        "a rifle at 600 units is aimed, not hip-fired"
    );
    assert_eq!(
        brain.released_between_thinks() & buttons::ATTACK,
        0,
        "an automatic keeps its trigger down between thinks"
    );
}

#[test]
fn a_semi_automatic_lets_the_trigger_up_between_thinks() {
    let mut field = Field::open(WeaponClass::Pistol);
    field.facts.fire_single = true;
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
    let cmds = fight(&mut brain, &mut field, [300.0, 0.0, 0.0], 20, 0.0);
    assert!(cmds.iter().any(|cmd| cmd.buttons & buttons::ATTACK != 0));
    assert_ne!(
        brain.released_between_thinks() & buttons::ATTACK,
        0,
        "held through the gap, a single-shot weapon fires once per fight"
    );
}

#[test]
fn a_sniper_holds_fire_until_the_scope_is_up() {
    let mut field = Field::open(WeaponClass::Sniper);
    field.facts.fire_single = true;
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Veteran);
    let hip = fight(&mut brain, &mut field, [1200.0, 0.0, 0.0], 20, 0.0);
    assert!(
        hip.iter()
            .skip(1)
            .all(|cmd| cmd.buttons & buttons::ADS != 0)
    );
    assert!(
        hip.iter().all(|cmd| cmd.buttons & buttons::ATTACK == 0),
        "no shot leaves a scoped rifle from the hip"
    );
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Veteran);
    let scoped = fight(&mut brain, &mut field, [1200.0, 0.0, 0.0], 20, 1.0);
    assert!(scoped.iter().any(|cmd| cmd.buttons & buttons::ATTACK != 0));
}

#[test]
fn a_knife_range_enemy_is_knifed_not_shot() {
    let mut field = Field::open(WeaponClass::Smg);
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
    let cmds = fight(&mut brain, &mut field, [40.0, 0.0, 0.0], 20, 0.0);
    assert!(
        cmds.iter()
            .any(|cmd| cmd.buttons & buttons::MELEE_CHARGE != 0)
    );
    assert!(cmds.iter().all(|cmd| cmd.buttons & buttons::ATTACK == 0));
    assert_ne!(brain.released_between_thinks() & buttons::MELEE_CHARGE, 0);
}

#[test]
fn a_bot_that_can_strafes_while_it_trades_shots_and_a_recruit_stands() {
    for (level, strafes) in [(SkillLevel::Regular, true), (SkillLevel::Recruit, false)] {
        let mut field = Field::open(WeaponClass::Assault);
        let mut brain = HostController::with_skill(CALM_SEED, level);
        let mut moved = false;
        for step in 0..20 {
            let tick = 1 + step * 2;
            let mut obs = observe(tick, me(1, &field), &field);
            obs.seen.push(enemy(
                9,
                [400.0, 0.0, 0.0],
                tick,
                KnowledgeSource::CurrentlySeen,
            ));
            brain.drive(&obs, &mut field, THINK_MS);
            let intent = brain.intent();
            if intent.move_mode == MoveMode::Walk {
                let goal = intent.move_goal.expect("a walk has somewhere to go");
                assert!(
                    goal[1].abs() > 40.0 && goal[0].abs() < 1.0,
                    "a strafe crosses the line of fire, it does not close it: {goal:?}"
                );
                moved = true;
            }
        }
        assert_eq!(moved, strafes, "{level:?}");
    }
}

#[test]
fn a_heard_enemy_is_walked_to_and_never_shot() {
    let mut field = Field::open(WeaponClass::Assault);
    field.walled = true;
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
    let spot = [800.0, 0.0, 0.0];
    for step in 0..10 {
        let tick = 1 + step * 2;
        let mut obs = observe(tick, me(1, &field), &field);
        obs.heard.push(enemy(9, spot, tick, KnowledgeSource::Heard));
        let cmd = brain.drive(&obs, &mut field, THINK_MS);
        assert_eq!(
            cmd.buttons & buttons::ATTACK,
            0,
            "a sound is a place to go, never a target"
        );
        assert_eq!(brain.task().kind, TaskKind::Investigate);
        assert_eq!(brain.intent().move_goal, Some(spot));
    }
}

#[test]
fn the_last_place_an_enemy_was_heard_is_where_the_hunt_goes_next() {
    let mut field = Field::open(WeaponClass::Assault);
    field.walled = true;
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
    let spot = [900.0, 300.0, 0.0];
    let mut obs = observe(1, me(1, &field), &field);
    obs.heard.push(enemy(9, spot, 1, KnowledgeSource::Heard));
    brain.drive(&obs, &mut field, THINK_MS);
    assert_eq!(brain.task().kind, TaskKind::Investigate);
    // One shot, then silence. The bot walks on; the contact goes stale and the
    // task falls back to hunting, which should carry on to the same place.
    let mut origin = [0.0f32; 3];
    let mut hunting = false;
    for step in 1..60 {
        if let Some(goal) = brain.intent().move_goal {
            let to = [goal[0] - origin[0], goal[1] - origin[1]];
            let len = (to[0] * to[0] + to[1] * to[1]).sqrt().max(1.0);
            origin[0] += to[0] / len * 15.0;
            origin[1] += to[1] / len * 15.0;
        }
        let obs = observe(
            1 + step * 2,
            SelfState {
                origin,
                ..me(1, &field)
            },
            &field,
        );
        brain.drive(&obs, &mut field, THINK_MS);
        if brain.task().kind == TaskKind::Hunt {
            hunting = true;
            break;
        }
    }
    assert!(hunting, "a stale sound does not hold the bot forever");
    assert_eq!(brain.intent().move_goal, Some(spot));
}

#[test]
fn damage_from_behind_turns_the_bot_to_face_it() {
    let mut field = Field::open(WeaponClass::Assault);
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
    let obs = observe(1, me(1, &field), &field);
    brain.drive(&obs, &mut field, THINK_MS);
    let mut last = UserCmd::default();
    for step in 1..12 {
        let obs = observe(
            1 + step * 2,
            SelfState {
                damage_event: 1,
                attacker_yaw: Some(180.0),
                health: 70,
                ..me(1, &field)
            },
            &field,
        );
        last = brain.drive(&obs, &mut field, THINK_MS);
    }
    let off = math_iw4::angle_subtract(yaw_of(&last), 180.0).abs();
    assert!(off < 12.0, "still {off} degrees off the attacker's bearing");
}

#[test]
fn every_bot_thinks_on_every_driven_tick_whatever_its_client_id() {
    // The host drives bots on ticks of one parity. Staggering thinks by client
    // id on top of that froze the intent of every id of the other parity.
    for id in [1, 2] {
        let mut field = Field::open(WeaponClass::Assault);
        let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
        for step in 0..8 {
            let tick = 1 + step * 2;
            let at = [600.0, 40.0 * step as f32, 0.0];
            let mut obs = observe(tick, me(id, &field), &field);
            obs.seen
                .push(enemy(9, at, tick, KnowledgeSource::CurrentlySeen));
            brain.drive(&obs, &mut field, THINK_MS);
            assert_eq!(
                brain.intent().look_at,
                Some(obs.seen[0].aim_point()),
                "client {id} is aiming at where the enemy was, at tick {tick}"
            );
        }
    }
}

#[test]
fn the_dead_are_forgotten_and_a_sound_is_not_a_sighting() {
    let field = Field::open(WeaponClass::Assault);
    let mut memory = Memory::default();
    let mut obs = observe(1, me(1, &field), &field);
    obs.seen.push(enemy(
        9,
        [300.0, 0.0, 0.0],
        1,
        KnowledgeSource::CurrentlySeen,
    ));
    obs.heard
        .push(enemy(8, [900.0, 0.0, 0.0], 1, KnowledgeSource::Heard));
    let events = memory.ingest(&obs);
    assert_eq!(events, vec![BotEvent::Spotted { id: ClientId(9) }]);
    assert_eq!(memory.currently_seen().count(), 1);
    let heard: Vec<_> = memory.last_seen().collect();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].id, ClientId(8));

    let mut obs = observe(3, me(1, &field), &field);
    obs.gone = vec![ClientId(9), ClientId(8)];
    memory.ingest(&obs);
    assert_eq!(memory.currently_seen().count(), 0);
    assert_eq!(memory.last_seen().count(), 0);
}

#[test]
fn a_low_magazine_is_topped_up_only_when_asked_and_only_in_the_quiet() {
    let field = Field::open(WeaponClass::Assault);
    let low = SelfState {
        ammo_clip: 8,
        ..me(1, &field)
    };
    let quiet = observe(1, low, &field);

    let mut skill = WeaponSkill::default();
    skill.update(&quiet, true, true);
    assert!(skill.wants_reload());

    let mut skill = WeaponSkill::default();
    skill.update(&quiet, true, false);
    assert!(!skill.wants_reload(), "not asked");

    let mut contested = observe(1, low, &field);
    contested.seen.push(enemy(
        9,
        [300.0, 0.0, 0.0],
        1,
        KnowledgeSource::CurrentlySeen,
    ));
    let mut skill = WeaponSkill::default();
    skill.update(&contested, true, true);
    assert!(!skill.wants_reload(), "an enemy is in sight");

    let full = observe(1, me(1, &field), &field);
    let mut skill = WeaponSkill::default();
    skill.update(&full, true, true);
    assert!(!skill.wants_reload(), "the magazine is full");
}

#[test]
fn hearing_is_gunfire_far_footsteps_near_and_radar_everywhere() {
    let far = 2000.0 * 2000.0;
    let near = 200.0 * 200.0;
    let running = 190.0 * 190.0;
    let creeping = 80.0 * 80.0;
    assert!(audible(false, true, 0.0, far, 1.0), "unsuppressed fire");
    assert!(!audible(false, false, running, far, 1.0), "distant steps");
    assert!(audible(false, false, running, near, 1.0), "steps close by");
    assert!(!audible(false, false, creeping, near, 1.0), "crouch-walk");
    assert!(audible(true, false, 0.0, far * 9.0, 1.0), "radar sweep");
    assert!(
        !audible(false, true, 0.0, far, 0.6),
        "a recruit's ears do not carry that far"
    );
}

#[test]
fn the_attacker_is_opposite_the_way_the_damage_travelled() {
    let off = |yaw_byte: u32, expected: f32| {
        let bearing = bearing_to_attacker(yaw_byte, 0).expect("directional damage");
        math_iw4::angle_subtract(bearing, expected).abs()
    };
    assert_eq!(bearing_to_attacker(255, 255), None);
    assert!(off(0, 180.0) < 0.01);
    // Damage travelling north (yaw 90) came from the south.
    assert!(off(64, 270.0) < 0.01);
    assert!(off(128, 0.0) < 0.01);
}

#[test]
fn bots_are_never_handed_the_riot_shield_class() {
    let mut picked = [0u32; 5];
    for seed in 0..40u64 {
        for client in 1..18 {
            for pick in 0..6 {
                let index = default_class_index(seed * 0x9E37, ClientId(client), pick);
                picked[usize::from(index)] += 1;
            }
        }
    }
    assert_eq!(picked[4], 0, "class 4 is the riot shield");
    assert!(
        picked[..4].iter().all(|count| *count > 0),
        "every fighting class turns up: {picked:?}"
    );
}

#[test]
fn a_crouched_enemy_is_aimed_at_lower_than_a_standing_one() {
    let standing = enemy(9, [0.0; 3], 1, KnowledgeSource::CurrentlySeen);
    let crouched = Contact {
        height: 40.0,
        ..standing
    };
    let unknown = Contact {
        height: 0.0,
        ..standing
    };
    assert!(crouched.aim_point()[2] < standing.aim_point()[2]);
    assert_eq!(unknown.aim_point(), standing.aim_point());
}

#[test]
fn weapon_classes_keep_their_ranges_in_order() {
    use WeaponClass::*;
    for class in [Assault, Smg, Lmg, Sniper, Shotgun, Pistol, Launcher] {
        assert!(class.fight_hold() < class.engage_max(), "{class:?}");
    }
    assert!(Shotgun.fight_hold() < Smg.fight_hold());
    assert!(Smg.fight_hold() < Assault.fight_hold());
    assert!(Assault.fight_hold() < Sniper.fight_hold());
    assert_eq!(WeaponClass::from_weap_class(1), Some(Sniper));
    assert_eq!(WeaponClass::from_weap_class(4), Some(Shotgun));
    assert_eq!(
        WeaponClass::from_weap_class(6),
        None,
        "a grenade is not a gun"
    );
}

/// Every route fails here: a walk never clears, so nothing is reachable.
struct Sealed(Field);

impl WorldQuery for Sealed {
    fn sight_ray(&mut self, a: [f32; 3], b: [f32; 3], c: ClientId) -> QueryResult<SightSample> {
        self.0.sight_ray(a, b, c)
    }

    fn shot_ray(&mut self, a: [f32; 3], b: [f32; 3], c: ClientId) -> QueryResult<SightSample> {
        self.0.shot_ray(a, b, c)
    }

    fn hull_trace(&mut self, start: [f32; 3], _: [f32; 3]) -> QueryResult<HullTrace> {
        Ok(HullTrace {
            fraction: 0.0,
            normal: [0.0, 0.0, 1.0],
            endpos: start,
            startsolid: false,
        })
    }

    fn weapon_class(&self, weapon: u16) -> WeaponClass {
        self.0.weapon_class(weapon)
    }

    fn weapon_facts(&self, weapon: u16) -> Option<WeaponFacts> {
        self.0.weapon_facts(weapon)
    }
}

#[test]
fn a_sound_with_no_route_to_it_is_given_up_on_not_stared_at() {
    let mut field = Field::open(WeaponClass::Assault);
    field.walled = true;
    let mut world = Sealed(field);
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
    let spot = [800.0, 0.0, 0.0];
    let mut investigating = 0;
    // The enemy keeps making noise from the same unreachable place.
    for step in 0..40 {
        let tick = 1 + step * 2;
        let mut obs = observe(tick, me(1, &world.0), &world.0);
        obs.heard.push(enemy(9, spot, tick, KnowledgeSource::Heard));
        brain.drive(&obs, &mut world, THINK_MS);
        investigating += u32::from(brain.task().kind == TaskKind::Investigate);
    }
    assert!(
        investigating <= 2,
        "held on an unreachable sound for {investigating} of 40 thinks"
    );
}

const FRAG: Offhand = Offhand {
    weapon: 31,
    ammo: 1,
    facts: ThrowFacts {
        speed: 900.0,
        speed_up: 120.0,
        kind: ThrowKind::Frag,
        fuse_ms: 3500,
        radius: 256.0,
    },
};

/// Drives a bot that keeps hearing one enemy it cannot see, and returns every
/// command it produced.
fn hear(brain: &mut HostController, field: &mut Field, at: [f32; 3], thinks: u32) -> Vec<UserCmd> {
    (0..thinks)
        .map(|step| {
            let tick = 1 + step * 2;
            let mut obs = observe(
                tick,
                SelfState {
                    lethal: Some(FRAG),
                    ..me(1, field)
                },
                field,
            );
            obs.heard.push(enemy(9, at, tick, KnowledgeSource::Heard));
            brain.drive(&obs, &mut *field, THINK_MS)
        })
        .collect()
}

#[test]
fn a_heard_enemy_in_reach_gets_one_grenade_pulled_then_thrown() {
    let mut field = Field::open(WeaponClass::Assault);
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Veteran);
    let cmds = hear(&mut brain, &mut field, [800.0, 0.0, 0.0], 400);
    let held: Vec<bool> = cmds
        .iter()
        .map(|cmd| cmd.buttons & buttons::FRAG != 0)
        .collect();
    let pulls = held.windows(2).filter(|pair| !pair[0] && pair[1]).count();
    assert_eq!(pulls, 1, "one lethal a life, and it is used");
    assert!(
        held.windows(2).any(|pair| pair[0] && !pair[1]),
        "a pin that is pulled is let go"
    );
    for cmd in cmds.iter().filter(|cmd| cmd.buttons & buttons::FRAG != 0) {
        assert_eq!(
            cmd.off_hand_index, FRAG.weapon,
            "the command names the grenade"
        );
        assert_eq!(cmd.buttons & buttons::ATTACK, 0);
        assert!(
            cmd.angles[0] < 0,
            "a throw at 800 units is lobbed above the horizon"
        );
    }
}

#[test]
fn no_grenade_is_thrown_too_close_through_a_wall_or_by_a_recruit() {
    let quiet = |cmds: &[UserCmd]| cmds.iter().all(|cmd| cmd.buttons & buttons::FRAG == 0);

    let mut field = Field::open(WeaponClass::Assault);
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Veteran);
    assert!(
        quiet(&hear(&mut brain, &mut field, [200.0, 0.0, 0.0], 200)),
        "inside its own blast"
    );

    let mut field = Field::open(WeaponClass::Assault);
    field.walled = true;
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Veteran);
    assert!(
        quiet(&hear(&mut brain, &mut field, [800.0, 0.0, 0.0], 200)),
        "the arc is blocked"
    );

    let mut field = Field::open(WeaponClass::Assault);
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Recruit);
    assert!(
        quiet(&hear(&mut brain, &mut field, [800.0, 0.0, 0.0], 200)),
        "recruits keep their grenades"
    );
}

/// Drives a bot holding an earned killstreak through `thinks` quiet thinks.
/// `world` stands in for the script: it sees each command and decides what the
/// bot observes next.
fn earn(
    kind: StreakKind,
    thinks: u32,
    mut world: impl FnMut(&UserCmd, &mut SelfState),
) -> Vec<UserCmd> {
    const STREAK: u16 = 77;
    let mut field = Field::open(WeaponClass::Assault);
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
    let mut state = SelfState {
        streak: Some(Streak {
            weapon: STREAK,
            kind,
        }),
        ..me(1, &field)
    };
    (0..thinks)
        .map(|step| {
            let obs = observe(1 + step * 2, state, &field);
            let cmd = brain.drive(&obs, &mut field, THINK_MS);
            world(&cmd, &mut state);
            cmd
        })
        .collect()
}

#[test]
fn an_instant_killstreak_is_called_in_by_selecting_its_weapon() {
    let cmds = earn(StreakKind::Instant, 30, |cmd, state| {
        // The script uses it the moment it is in hand, and takes it back.
        if cmd.weapon == 77 {
            state.streak = None;
        }
    });
    assert!(cmds.iter().any(|cmd| cmd.weapon == 77));
    assert_eq!(
        cmds.last().unwrap().weapon,
        RIFLE,
        "and the rifle comes back"
    );
}

#[test]
fn an_airstrike_confirms_the_map_and_a_marker_is_thrown() {
    let cmds = earn(StreakKind::Location, 40, |cmd, state| {
        if cmd.weapon == 77 {
            state.locating = true;
        }
        if cmd.buttons & buttons::LOCATION_SELECT != 0 {
            state.locating = false;
            state.streak = None;
        }
    });
    assert!(
        cmds.iter()
            .any(|cmd| cmd.buttons & buttons::LOCATION_SELECT != 0)
    );

    let cmds = earn(StreakKind::Thrown, 40, |cmd, state| {
        if cmd.weapon == 77 {
            state.weapon = 77;
        }
        if cmd.buttons & buttons::ATTACK != 0 {
            state.weapon = RIFLE;
            state.streak = None;
        }
    });
    assert!(cmds.iter().any(|cmd| cmd.buttons & buttons::ATTACK != 0));
}

#[test]
fn a_crewed_killstreak_is_left_alone_and_a_failed_one_is_not_spammed() {
    let cmds = earn(StreakKind::Crewed, 40, |_, _| {});
    assert!(cmds.iter().all(|cmd| cmd.weapon == RIFLE));

    // The script never takes it: the bot stops asking and goes back to its gun.
    let cmds = earn(StreakKind::Instant, 120, |_, _| {});
    let asking = cmds.iter().filter(|cmd| cmd.weapon == 77).count();
    assert!(asking > 0 && asking < 70, "asked on {asking} of 120 thinks");
    assert_eq!(cmds.last().unwrap().weapon, RIFLE);
}

#[test]
fn a_care_package_is_opened_with_use_and_given_up_on_if_it_never_opens() {
    let mut field = Field::open(WeaponClass::Assault);
    let mut brain = HostController::with_skill(CALM_SEED, SkillLevel::Regular);
    let crate_drop = ModeObjective {
        id: 4,
        touching: true,
        use_button: true,
        radius: 56.0,
        ..ModeObjective::at([20.0, 0.0, 0.0])
    };
    let mut used = 0;
    let mut last = UserCmd::default();
    for step in 0..400 {
        let mut obs = observe(1 + step * 2, me(1, &field), &field);
        obs.objectives.push(crate_drop);
        obs.objective = Some(crate_drop);
        last = brain.drive(&obs, &mut field, THINK_MS);
        used += u32::from(last.buttons & buttons::USE != 0);
    }
    assert!(used > 20, "use was held on the crate: {used} thinks");
    assert_eq!(
        last.buttons & buttons::USE,
        0,
        "a crate that never opens is walked away from"
    );
}
