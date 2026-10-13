use client_extensions::modding::orbit_hitbox::HitboxPolicy;
use client_world::ActorSnapshot;

pub(crate) fn bounds(
    actor: &ActorSnapshot,
    boxes: impl IntoIterator<Item = ([f32; 3], [f32; 3])>,
    policy: Option<HitboxPolicy>,
    origin: [f32; 3],
) -> impl Iterator<Item = ([f32; 3], [f32; 3])> {
    let policy = policy.filter(|policy| {
        policy.applies(
            matches!(actor.kind, protocol::ActorKind::Player { .. }),
            actor.position,
            origin,
        )
    });
    boxes
        .into_iter()
        .map(move |(min, max)| policy.map_or((min, max), |policy| policy.bounds(min, max)))
}

pub(super) fn outlines(
    gameplay: &super::gameplay::GameplayContext,
    policy: Option<HitboxPolicy>,
    mount: Option<i64>,
    origin: [f32; 3],
) -> Vec<[[f32; 3]; 2]> {
    let Some(policy) = policy.filter(|policy| policy.outlines) else {
        return Vec::new();
    };
    let Some(stream) = gameplay
        .world
        .as_ref()
        .and_then(|world| world.stream.as_ref())
    else {
        return Vec::new();
    };
    let authority = stream.authority();
    let partial = gameplay.actor_partial.as_ref().map_or(0.0, |tick| tick.0);
    let mut actors: Vec<_> = authority
        .remote_actors()
        .filter(|actor| {
            Some(actor.unique_id) != mount
                && gameplay::melee::actor_is_pickable(actor)
                && authority
                    .actor_health_by_unique(actor.unique_id)
                    .is_none_or(|(health, _)| health > 0.0)
                && policy.applies(
                    matches!(actor.kind, protocol::ActorKind::Player { .. }),
                    actor.position,
                    origin,
                )
        })
        .collect();
    actors.sort_by(|left, right| {
        let distance = |position: [f32; 3]| {
            (0..3)
                .map(|axis| (position[axis] - origin[axis]).powi(2))
                .sum::<f32>()
        };
        distance(left.position)
            .total_cmp(&distance(right.position))
            .then(left.runtime_id.cmp(&right.runtime_id))
    });
    actors
        .into_iter()
        .take(mod_api::MAX_OBSERVED_ACTORS)
        .flat_map(|actor| {
            let position = actor
                .interpolated_position(partial)
                .unwrap_or(actor.position);
            actor
                .hit_boxes_at(position)
                .take(mod_api::MAX_OBSERVED_ACTOR_BOXES)
                .map(|(min, max)| {
                    let (min, max) = policy.bounds(min, max);
                    [min, max]
                })
                .collect::<Vec<_>>()
        })
        .collect()
}
