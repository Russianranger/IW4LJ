//! Aiming a thrown grenade. The throw leaves the eye along the view at the
//! weapon's speed, with a fixed lift added straight up, and falls under the
//! missile gravity; this finds the view angle that lands it on a point.

use crate::observation::ThrowFacts;
use crate::query::{QueryResult, SightSample, WorldQuery};
use sim::ClientId;

/// Half the acceleration a thrown missile falls at.
const HALF_GRAVITY: f32 = 400.0;
/// The view angles tried, in degrees above the horizon. A throw is never
/// lobbed steeper than this: a high arc is slow, and indoors it is a ceiling.
const LOWEST_DEG: f32 = -25.0;
const HIGHEST_DEG: f32 = 50.0;
const STEP_DEG: f32 = 0.5;
/// The arc is checked for walls at this many points along it.
const ARC_SAMPLES: usize = 4;

/// A solved throw: where to look, and how long the grenade is in the air.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Throw {
    /// A point along the view to turn to, well out from the eye.
    pub look_at: [f32; 3],
    pub flight_s: f32,
    yaw_rad: f32,
    lift_rad: f32,
}

/// The flattest throw from `eye` that comes down on `target`. `None` when the
/// weapon cannot reach it inside the angles a throw is allowed.
pub fn solve(eye: [f32; 3], target: [f32; 3], facts: &ThrowFacts) -> Option<Throw> {
    let dx = target[0] - eye[0];
    let dy = target[1] - eye[1];
    let dist = (dx * dx + dy * dy).sqrt();
    if dist < 1.0 || facts.speed <= 1.0 {
        return None;
    }
    let rise = target[2] - eye[2];
    // Height above the target when the grenade has covered the ground between.
    let miss = |lift_deg: f32| {
        let lift = lift_deg.to_radians();
        let level = facts.speed * lift.cos();
        if level <= 1.0 {
            return None;
        }
        let t = dist / level;
        let up = facts.speed * lift.sin() + facts.speed_up;
        Some((up * t - HALF_GRAVITY * t * t - rise, t))
    };
    let mut below: Option<(f32, f32, f32)> = None;
    let mut lift_deg = LOWEST_DEG;
    while lift_deg <= HIGHEST_DEG {
        let (error, t) = miss(lift_deg)?;
        if error >= 0.0 {
            // Falling short at the last angle and long at this one: the answer
            // is between them.
            let (lift, flight_s) = match below {
                Some((low_deg, low_error, low_t)) => {
                    let span: f32 = error - low_error;
                    let along = if span.abs() > f32::EPSILON {
                        -low_error / span
                    } else {
                        0.0
                    };
                    (
                        low_deg + (lift_deg - low_deg) * along,
                        low_t + (t - low_t) * along,
                    )
                }
                // Already long at the flattest angle: the target is close and
                // below. Throw flat rather than refuse.
                None => (lift_deg, t),
            };
            let yaw_rad = dy.atan2(dx);
            let lift_rad = lift.to_radians();
            let reach = 512.0;
            return Some(Throw {
                look_at: [
                    eye[0] + yaw_rad.cos() * lift_rad.cos() * reach,
                    eye[1] + yaw_rad.sin() * lift_rad.cos() * reach,
                    eye[2] + lift_rad.sin() * reach,
                ],
                flight_s,
                yaw_rad,
                lift_rad,
            });
        }
        below = Some((lift_deg, error, t));
        lift_deg += STEP_DEG;
    }
    None
}

impl Throw {
    /// Where the grenade is `t` seconds after leaving `eye`.
    pub fn at(&self, eye: [f32; 3], facts: &ThrowFacts, t: f32) -> [f32; 3] {
        let level = facts.speed * self.lift_rad.cos();
        let up = facts.speed * self.lift_rad.sin() + facts.speed_up;
        [
            eye[0] + self.yaw_rad.cos() * level * t,
            eye[1] + self.yaw_rad.sin() * level * t,
            eye[2] + up * t - HALF_GRAVITY * t * t,
        ]
    }

    /// Whether the arc gets most of the way there without meeting a wall. A
    /// throw that would stop early comes back at the thrower. The last stretch
    /// is not checked: it ends in the ground by design.
    pub fn arc_is_open(
        &self,
        eye: [f32; 3],
        facts: &ThrowFacts,
        world: &mut impl WorldQuery,
        thrower: ClientId,
    ) -> QueryResult<bool> {
        let mut from = eye;
        for step in 1..ARC_SAMPLES {
            let to = self.at(eye, facts, self.flight_s * step as f32 / ARC_SAMPLES as f32);
            if matches!(
                world.sight_ray(from, to, thrower)?,
                SightSample::Blocked { .. }
            ) {
                return Ok(false);
            }
            from = to;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::ThrowKind;

    const FRAG: ThrowFacts = ThrowFacts {
        speed: 900.0,
        speed_up: 120.0,
        kind: ThrowKind::Frag,
        fuse_ms: 3500,
        radius: 256.0,
    };

    #[test]
    fn a_solved_throw_lands_on_the_target() {
        let eye = [100.0, -50.0, 60.0];
        for target in [
            [700.0, 200.0, 0.0],
            [100.0, 850.0, 140.0],
            [-500.0, -400.0, -90.0],
            [380.0, -50.0, 0.0],
        ] {
            let throw = solve(eye, target, &FRAG).expect("in reach");
            let landed = throw.at(eye, &FRAG, throw.flight_s);
            for axis in 0..3 {
                assert!(
                    (landed[axis] - target[axis]).abs() < 6.0,
                    "{target:?}: landed {landed:?}"
                );
            }
        }
    }

    #[test]
    fn a_further_target_takes_a_higher_longer_throw() {
        let eye = [0.0, 0.0, 60.0];
        let near = solve(eye, [400.0, 0.0, 0.0], &FRAG).unwrap();
        let far = solve(eye, [1000.0, 0.0, 0.0], &FRAG).unwrap();
        assert!(far.look_at[2] > near.look_at[2]);
        assert!(far.flight_s > near.flight_s);
    }

    #[test]
    fn a_target_out_of_reach_is_refused() {
        let eye = [0.0, 0.0, 60.0];
        assert_eq!(solve(eye, [6000.0, 0.0, 0.0], &FRAG), None);
        assert_eq!(solve(eye, [300.0, 0.0, 2000.0], &FRAG), None);
        assert_eq!(solve(eye, eye, &FRAG), None);
    }
}
