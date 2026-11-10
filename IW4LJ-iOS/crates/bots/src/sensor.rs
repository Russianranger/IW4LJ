use math_iw4::{angle_normalize_360, angle_vectors};
use sim::{ClientId, ClientLifecycle, RadarMode, Snapshot};

use crate::observation::{
    BotObservation, Contact, DEFAULT_OBJECTIVE_RADIUS, KnowledgeSource, ModeObjective, Offhand,
    SelfState, Streak, Visibility, WeaponAction, WeaponSlot,
};
use crate::query::{QuerySubsystem, SightSample, WorldQuery};
use crate::skill::BotSkill;

pub const SIGHT_RANGE_IN: f32 = 5906.0;
const PROBE_CHEST_Z: f32 = 48.0;
const PROBE_HEAD_Z: f32 = 64.0;
const PROBE_FEET_Z: f32 = 8.0;
/// Perception's share of the shared quota, so routing and firing keep theirs.
const SIGHT_PROBES: u32 = 9;
/// Inside this an enemy is noticed whichever way the bot faces, given a clear
/// line: nobody walks up to a player's shoulder unnoticed.
const CLOSE_AWARE_IN: f32 = 140.0;
/// Unsuppressed gunfire. It also puts a dot on the minimap, so the position is
/// known without a line of sight.
const GUNFIRE_HEAR_IN: f32 = 2800.0;
/// Running and sprinting footsteps. Crouch-walking and aiming are quieter than
/// the speed below and are not heard.
const FOOTSTEP_HEAR_IN: f32 = 260.0;
const FOOTSTEP_SPEED: f32 = 150.0;
/// How close the feet have to be to a care package to open it.
const CRATE_USE_RADIUS: f32 = 56.0;

pub fn observe(
    snapshot: &Snapshot,
    bot: ClientId,
    world: &mut impl WorldQuery,
) -> Option<BotObservation> {
    observe_focused(snapshot, bot, world, None)
}

/// `focus` is the client the bot has already committed to. It is probed before
/// the rotation spends the same allowance on acquisition candidates, so a
/// committed target is the last thing perception gives up, not the first.
pub fn observe_focused(
    snapshot: &Snapshot,
    bot: ClientId,
    world: &mut impl WorldQuery,
    focus: Option<ClientId>,
) -> Option<BotObservation> {
    observe_with(snapshot, bot, world, focus, &BotSkill::default())
}

/// The full observation: sight inside the skill's field of view, then what the
/// bot may know without seeing it — gunfire, footsteps, a radar sweep and the
/// direction it was last hurt from.
pub fn observe_with(
    snapshot: &Snapshot,
    bot: ClientId,
    world: &mut impl WorldQuery,
    focus: Option<ClientId>,
    skill: &BotSkill,
) -> Option<BotObservation> {
    let meta = snapshot.meta.for_client(bot)?;
    let ps = snapshot
        .players
        .iter()
        .find(|(id, _)| *id == bot)
        .map(|(_, ps)| ps)?;
    let team = meta.client_state_team;
    world.enter(QuerySubsystem::Perception);
    let self_state = SelfState {
        life_sequence: meta.life_sequence,
        id: bot,
        lifecycle: meta.lifecycle,
        origin: ps.origin,
        viewangles: ps.viewangles,
        delta_angles: ps.delta_angles,
        view_height: ps.view_height_current,
        stance: crate::observation::Stance::from_view_height(ps.view_height_current),
        health: ps.health,
        weapon: ps.weapon as u16,
        weapon_class: world.weapon_class(ps.weapon as u16),
        weapon_action: WeaponAction::from_weaponstate(ps.weaponstate_primary),
        ammo_clip: meta.ammo_clip,
        ammo_stock: meta.ammo_stock,
        team,
        velocity: ps.velocity,
        ads: ps.f_weapon_pos_frac,
        damage_event: ps.damage_event,
        attacker_yaw: attacker_yaw(ps),
        radar: meta.radar != RadarMode::Off && !meta.radar_blocked,
        lethal: own_offhand(meta, world, |loadout| loadout.lethal),
        tactical: own_offhand(meta, world, |loadout| loadout.tactical),
        streak: earned_streak(ps, world),
        locating: meta.location_selection.is_some(),
    };
    let inventory = own_inventory(ps, meta, world);
    let mut seen = Vec::new();
    let mut heard = Vec::new();
    let mut gone = Vec::new();
    let mut unsensed = Vec::new();
    if meta.lifecycle != ClientLifecycle::Alive {
        // A bot that is not alive performs no probe, so it makes no negative
        // observation either.
        unsensed.extend(
            snapshot
                .players
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| *id != bot),
        );
    } else {
        let eye = [
            ps.origin[0],
            ps.origin[1],
            ps.origin[2]
                + if ps.view_height_current > 1.0 {
                    ps.view_height_current
                } else {
                    60.0
                },
        ];
        let (forward, _, _) = angle_vectors(ps.viewangles);
        let half = (skill.fov_deg * 0.5).to_radians().cos();
        // Rotate probes so occluded clients at the front of the snapshot cannot
        // monopolize perception.
        let mut probes = SIGHT_PROBES;
        let count = snapshot.players.len();
        let start = (snapshot.tick.0 as usize / 2 + bot.0 as usize) % count.max(1);
        for index in probe_order(snapshot, start, count, focus) {
            let (id, other) = &snapshot.players[index];
            if *id == bot {
                continue;
            }
            let Some(other_meta) = snapshot.meta.for_client(*id) else {
                continue;
            };
            if snapshot.meta.kind.is_team() && other_meta.client_state_team == team {
                continue;
            }
            if other_meta.lifecycle != ClientLifecycle::Alive {
                gone.push(*id);
                continue;
            }
            let range2 = dist2(ps.origin, other.origin);
            let heard_as = Contact {
                id: *id,
                origin: other.origin,
                source: KnowledgeSource::Heard,
                seen_tick: snapshot.tick.0,
                confidence: 0.5,
                height: other.view_height_current,
            };
            // Out of range or outside the cone is a complete answer on its own;
            // only a client that needed a probe it did not get is unsensed.
            if range2 > CLOSE_AWARE_IN * CLOSE_AWARE_IN
                && !in_range_and_fov(ps.origin, other.origin, forward, half)
            {
                if placed_unseen(world, self_state.radar, other, range2, skill.hearing) {
                    heard.push(heard_as);
                }
                continue;
            }
            if probes == 0 {
                unsensed.push(*id);
                continue;
            }
            match probe_visible(world, bot, eye, other.origin, *id, &mut probes) {
                Visibility::Seen => seen.push(Contact {
                    source: KnowledgeSource::CurrentlySeen,
                    confidence: 1.0,
                    ..heard_as
                }),
                Visibility::NotSeen => {
                    if placed_unseen(world, self_state.radar, other, range2, skill.hearing) {
                        heard.push(heard_as);
                    }
                }
                Visibility::Unknown => unsensed.push(*id),
            }
        }
    }
    let objectives = public_objectives(snapshot, bot, team);
    Some(BotObservation {
        tick: snapshot.tick.0,
        time_ms: snapshot.tick.0 as i32 * 50,
        self_state,
        inventory,
        seen,
        heard,
        gone,
        unsensed,
        events: Vec::new(),
        objective: objectives
            .iter()
            .copied()
            .min_by(|a, b| dist2(ps.origin, a.origin).total_cmp(&dist2(ps.origin, b.origin))),
        objectives,
    })
}

fn placed_unseen(
    world: &impl WorldQuery,
    radar: bool,
    other: &playerstate_iw4::PlayerState,
    range2: f32,
    hearing: f32,
) -> bool {
    let firing = WeaponAction::from_weaponstate(other.weaponstate_primary) == WeaponAction::Firing;
    audible(
        radar,
        firing && !world.weapon_silenced(other.weapon as u16),
        other.velocity[0] * other.velocity[0] + other.velocity[1] * other.velocity[1],
        range2,
        hearing,
    )
}

/// Whether an enemy that is not in sight is still placed: by a radar sweep, by
/// firing a weapon without a suppressor, or by running close enough to hear.
/// Ranges and speed are squared.
pub(crate) fn audible(
    radar: bool,
    loud_fire: bool,
    speed2: f32,
    range2: f32,
    hearing: f32,
) -> bool {
    if radar {
        return true;
    }
    let gunfire = GUNFIRE_HEAR_IN * hearing;
    if loud_fire && range2 <= gunfire * gunfire {
        return true;
    }
    let steps = FOOTSTEP_HEAR_IN * hearing;
    range2 <= steps * steps && speed2 >= FOOTSTEP_SPEED * FOOTSTEP_SPEED
}

fn attacker_yaw(ps: &playerstate_iw4::PlayerState) -> Option<f32> {
    bearing_to_attacker(ps.damage_yaw, ps.damage_pitch)
}

/// World yaw from the bot toward the source of its last directional damage.
/// The player state stores the direction the damage travelled, packed to a
/// byte; 255/255 is the engine's marker for damage with no direction.
pub(crate) fn bearing_to_attacker(damage_yaw: u32, damage_pitch: u32) -> Option<f32> {
    if damage_yaw == 255 && damage_pitch == 255 {
        return None;
    }
    Some(angle_normalize_360(
        damage_yaw as f32 * (360.0 / 256.0) + 180.0,
    ))
}

fn in_range_and_fov(from: [f32; 3], to: [f32; 3], forward: [f32; 3], min_dot: f32) -> bool {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    if len2 > SIGHT_RANGE_IN * SIGHT_RANGE_IN || len2 <= 0.0001 {
        return false;
    }
    let len = len2.sqrt();
    let dir = [d[0] / len, d[1] / len, d[2] / len];
    dir[0] * forward[0] + dir[1] * forward[1] + dir[2] * forward[2] >= min_dot
}

/// A grenade slot of the class the bot spawned with, with the ammunition the
/// match state holds for it. A slot the match keeps no count for is taken to
/// hold one; the controller's own per-life limit keeps that honest.
fn own_offhand(
    meta: &sim::ClientSnapshotMeta,
    world: &impl WorldQuery,
    slot: impl Fn(&sim::LoadoutSpec) -> u32,
) -> Option<Offhand> {
    let id = slot(meta.loadout.as_ref()?);
    let weapon = u16::try_from(id).ok().filter(|weapon| *weapon != 0)?;
    let facts = world.throw_facts(weapon)?;
    let ammo = meta
        .ammo_by_weapon
        .iter()
        .find(|(owned, _, _)| *owned == id)
        .map_or(1, |(_, clip, stock)| clip + stock);
    (ammo > 0).then_some(Offhand {
        weapon,
        ammo,
        facts,
    })
}

/// The killstreak the script has put in the fourth action slot, which is where
/// a player finds it too.
fn earned_streak(ps: &playerstate_iw4::PlayerState, world: &impl WorldQuery) -> Option<Streak> {
    const SLOT: usize = 3;
    const SLOT_HOLDS_WEAPON: i32 = 1;
    if ps.action_slot_type[SLOT] != SLOT_HOLDS_WEAPON {
        return None;
    }
    let weapon = u16::try_from(ps.action_slot_param[SLOT]).ok()?;
    Some(Streak {
        weapon,
        kind: world.streak_kind(weapon)?,
    })
}

/// The bot's own carried weapons with the ammunition the match state holds for
/// them. Enemy inventories and positions are never read here.
fn own_inventory(
    ps: &playerstate_iw4::PlayerState,
    meta: &sim::ClientSnapshotMeta,
    world: &impl WorldQuery,
) -> Vec<WeaponSlot> {
    ps.weapons
        .iter()
        .copied()
        .filter(|&id| id > 0)
        .filter_map(|id| {
            let weapon = u16::try_from(id).ok()?;
            let facts = world.weapon_facts(weapon)?;
            let (clip, stock) = meta
                .ammo_by_weapon
                .iter()
                .find(|(owned, _, _)| *owned == id as u32)
                .map_or((0, 0), |(_, clip, stock)| (*clip, *stock));
            Some(WeaponSlot {
                weapon,
                class: world.weapon_class(weapon),
                facts,
                clip,
                stock,
            })
        })
        .collect()
}

/// Snapshot indices in the order perception spends its allowance: the committed
/// target first, then the tick's rotation.
fn probe_order(
    snapshot: &Snapshot,
    start: usize,
    count: usize,
    focus: Option<ClientId>,
) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count).map(|offset| (start + offset) % count).collect();
    if let Some(focus) = focus
        && let Some(at) = order
            .iter()
            .position(|index| snapshot.players[*index].0 == focus)
    {
        let first = order.remove(at);
        order.insert(0, first);
    }
    order
}

fn probe_visible(
    world: &mut impl WorldQuery,
    bot: ClientId,
    eye: [f32; 3],
    origin: [f32; 3],
    target: ClientId,
    remaining: &mut u32,
) -> Visibility {
    let probes = [
        [origin[0], origin[1], origin[2] + PROBE_CHEST_Z],
        [origin[0], origin[1], origin[2] + PROBE_HEAD_Z],
        [origin[0], origin[1], origin[2] + PROBE_FEET_Z],
    ];
    // One positive probe settles the question; a negative one settles it only
    // when every probe actually ran and came back classified.
    let mut complete = true;
    for probe in probes {
        if *remaining == 0 {
            return Visibility::Unknown;
        }
        *remaining -= 1;
        match world.sight_ray(eye, probe, bot) {
            Ok(SightSample::Clear) => return Visibility::Seen,
            Ok(SightSample::HitPlayer { client, .. }) if client == target => {
                return Visibility::Seen;
            }
            Err(_) => {
                // The shared quota is gone; stop attributing denials to this bot.
                *remaining = 0;
                return Visibility::Unknown;
            }
            Ok(SightSample::Unknown) => complete = false,
            Ok(_) => {}
        }
    }
    if complete {
        Visibility::NotSeen
    } else {
        Visibility::Unknown
    }
}

fn public_objectives(snapshot: &Snapshot, bot: ClientId, team: i32) -> Vec<ModeObjective> {
    let Some(team) = gamemode_iw4::Team::from_packed_u8(team as u8) else {
        return Vec::new();
    };
    let feet = snapshot
        .players
        .iter()
        .find(|(id, _)| *id == bot)
        .map(|(_, p)| p.origin);
    snapshot
        .meta
        .objectives
        .compass
        .iter()
        .filter(|o| {
            o.shows_to(team, bot.0)
                && !o.icon.contains("defend")
                && !o.icon.starts_with("compassping")
        })
        .map(|o| {
            // A care package is marked with the ammo icon. It is opened by
            // standing at it and holding use, not by standing near it.
            let crate_drop = o.icon.contains("objpoint_ammo");
            let radius = if crate_drop {
                CRATE_USE_RADIUS
            } else {
                DEFAULT_OBJECTIVE_RADIUS
            };
            ModeObjective {
                id: u32::from(o.index),
                origin: o.origin,
                touching: feet.is_some_and(|f| {
                    dist2([f[0], f[1], 0.0], [o.origin[0], o.origin[1], 0.0]) <= radius * radius
                }),
                use_button: crate_drop,
                radius,
                ..ModeObjective::at(o.origin)
            }
        })
        .collect()
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}
