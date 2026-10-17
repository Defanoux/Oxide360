//! Components (Fusion's): named groups of bodies, so a design can hold
//! several parts. A body is in at most one component; the rest are the
//! design's own (the root). Joints move components as wholes.

use crate::doc::BodyId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub bodies: Vec<BodyId>,
}

/// The component body `id` is in.
pub fn owner(components: &[Component], id: BodyId) -> Option<usize> {
    components.iter().position(|c| c.bodies.contains(&id))
}

/// A new component of `bodies` (taken out of any other one), named
/// "Component1", "Component2"… Its index.
pub fn create(components: &mut Vec<Component>, bodies: &[BodyId]) -> usize {
    for c in components.iter_mut() {
        c.bodies.retain(|b| !bodies.contains(b));
    }
    let mut n = components.len() + 1;
    while components.iter().any(|c| c.name == format!("Component{n}")) {
        n += 1;
    }
    let mut own: Vec<BodyId> = Vec::new();
    for b in bodies {
        if !own.contains(b) {
            own.push(*b);
        }
    }
    components.push(Component { name: format!("Component{n}"), bodies: own });
    components.len() - 1
}

/// Bodies were renumbered or deleted (`map`: where each one went, None:
/// gone). Components keep the rest.
pub fn remap(components: &mut [Component], map: impl Fn(BodyId) -> Option<BodyId>) {
    for c in components {
        c.bodies = c.bodies.iter().filter_map(|b| map(*b)).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Feature;

    fn id(k: usize) -> BodyId {
        BodyId { feature: Feature::Extrude(k), piece: 0 }
    }

    #[test]
    fn bodies_belong_to_one_component_at_most() {
        let mut cs = Vec::new();
        assert_eq!(create(&mut cs, &[id(0), id(1), id(1)]), 0);
        assert_eq!(cs[0].bodies, vec![id(0), id(1)], "no body twice");
        // Body 1 into a new component: it leaves the first.
        assert_eq!(create(&mut cs, &[id(1), id(2)]), 1);
        assert_eq!((cs[0].bodies.clone(), cs[1].name.as_str()), (vec![id(0)], "Component2"));
        assert_eq!(owner(&cs, id(2)), Some(1));
        assert_eq!(owner(&cs, id(5)), None);
        // Names don't repeat when one was renamed onto the next number.
        cs[0].name = "Component3".into();
        create(&mut cs, &[id(7)]);
        assert_eq!(cs[2].name, "Component4");
    }

    #[test]
    fn deleted_bodies_leave_their_component() {
        let mut cs = Vec::new();
        create(&mut cs, &[id(0), id(1), id(2)]);
        // Extrude 1 deleted: its body goes, extrude 2's renumbers.
        remap(&mut cs, |b| b.after_delete(false, 1));
        assert_eq!(cs[0].bodies, vec![id(0), id(1)]);
    }
}
