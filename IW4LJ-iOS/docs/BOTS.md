# `bots` — host AI

Authority tick: observation → memory → utility → task → route → motor → UserCmd.
Bots are driven on every second tick (10 Hz); the tick between resends the last
command with the press-to-act buttons released.

`bot skill recruit|regular|hardened|veteran` sets every bot's difficulty and is
saved to `$HOME/bot_skill.txt` (`IW4L_BOT_SKILL` overrides). `skill.rs` is the
one table: reaction, aim error and settling, turn rate, view cone, hearing.

`bot dummy [N]` creates bots with no controller or perception queries. They keep
their aim and weapon, accept `bot tp/give/fire`, and take damage and respawn normally.
They retain normal bot loadouts/perks; `bot hold off` leaves dummies idle.
Bots submit `TickInput.cmds` like players — [`SIM-STEP.md`](SIM-STEP.md).

| file | owns |
|---|---|
| `observation.rs` / `memory.rs` | what a bot may know, and for how long |
| `sensor.rs` | visibility probes and the rotation that spends them; hearing, radar and the damage bearing |
| `skill.rs` | the difficulty table |
| `controller.rs` / `task.rs` / `intent.rs` | scores, task choice, stages |
| `nav.rs` | the baked graph, A\*, resumable route requests |
| `query.rs` | the shared collision budget every subsystem draws on |
| `weapon.rs` / `motor.rs` | the weapon channel; aim and movement |

## The three rules that shape the rest

**A negative needs a probe that ran.** Visibility answers seen, evaluated and
not seen, or not evaluated. Only the second ends a contact; the third holds it,
briefly and without coordinates. Firing needs a positive observation of that
client, fresh on the tick the command goes out.

**A denied query is not a collision.** Perception, connectors, execution and
combat share one 96-query quota. A query that ran reports what it found even on
the last token; one that never ran comes back as denial, so no refusal is ever
fabricated from an empty budget.

**Work survives a tick that is granted nothing.** A route request owns its
phase — approach, attachments, search, validation — and every cursor inside it
is resumable, so a zero allocation repeats nothing and A\* keeps its frontier
under the shared 2048-expansion quota. Answers that follow from graph topology
alone are cached per request key until the graph changes; collision-dependent
refusals are rechecked.

**A sound is a place, never a target.** Unsuppressed gunfire (the minimap dot),
running footsteps close by and a radar sweep place an enemy without a line of
sight: a position to walk to, never a shot. Damage from out of sight turns the
bot to the bearing the damage indicator would draw; sight has to do the rest.

## Movement and combat

The graph bakes world-aligned tiles at 48 units, refined to 24 near missing
support or a height change, with directed Walk, Drop and BreakGlass edges that
keep their entry and exit through execution. A fight moves to a *position*: one
baked support per sector around the threat, checked for line of fire and
exposure, then reached through the ordinary route executor — reachability is
never inferred from a component label.

The engine's `weapClass` gives each weapon a range to fight at, a furthest range
worth standing and shooting from, and a range past which the sights come up.
Inside them a bot stands and strafes; outside it closes. Anything but full auto
lets the trigger up between shots, long automatic fire is burst, a scope is
waited for, knife reach is knifed. Aim wanders inside the skill's error and
tightens on a held target. With nothing in sight a bot walks to where an enemy
was last seen or heard, and roams only when it knows of none.

Not done: grenades and equipment, killstreak use, mantling and ladders.

`HostController::decision()` exposes scores, stage and objective; the periodic
aggregate adds per-subsystem query counts and stuck bots. Both read live from a
running match, which is where bot behaviour is judged: `make map mp_boneyard`
([`RUN.md`](RUN.md)) and watch the aggregate.
