//! Body colours: a default for each body by the order they were made in, picked
//! to be easy to tell apart without fighting the colours the interface uses
//! (the yellow dimensions, the orange sketch, the blue selection).

/// Muted tones. The first is the single-body colour.
pub const PALETTE: [[u8; 3]; 8] = [
    [170, 180, 195], // steel grey
    [150, 190, 165], // sage
    [200, 165, 150], // clay
    [175, 150, 210], // lavender
    [210, 195, 120], // sand
    [120, 190, 205], // sky teal
    [200, 150, 175], // dusty rose
    [160, 170, 140], // olive
];

/// The default colour of the `rank`th body made (wraps round).
pub fn default_color(rank: usize) -> [u8; 3] {
    PALETTE[rank % PALETTE.len()]
}

/// Which default colour a body gets: its place, oldest first, among the bodies
/// there are now (`order` sorts them: when its feature is in the timeline, then
/// which piece). No two bodies that exist together share a default (up to the
/// palette's size), however the timeline is reordered.
pub fn rank<K: PartialOrd>(all: impl IntoIterator<Item = K>, me: &K) -> usize {
    all.into_iter().filter(|k| k < me).count()
}

/// Perceived brightness 0..=255, to keep the palette in one band.
#[cfg(test)]
fn luma(c: [u8; 3]) -> f32 {
    0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_is_distinct_and_evenly_bright() {
        for (i, a) in PALETTE.iter().enumerate() {
            for b in &PALETTE[i + 1..] {
                let d: i32 = a.iter().zip(b).map(|(x, y)| (*x as i32 - *y as i32).abs()).sum();
                assert!(d >= 40, "{a:?} and {b:?} look too alike");
            }
            // Light enough to read on the dark background, soft enough not to
            // outshine the sketch colours.
            assert!((150.0..=200.0).contains(&luma(*a)), "{a:?}: {}", luma(*a));
        }
    }

    #[test]
    fn bodies_that_exist_together_never_share_a_default() {
        // (timeline position, piece): an extrude made two bodies, a later one a third.
        let bodies = [(0usize, 0usize), (0, 1), (1, 0)];
        let ranks: Vec<usize> = bodies.iter().map(|b| rank(bodies, b)).collect();
        assert_eq!(ranks, [0, 1, 2]);
        // The two-body extrude dragged after the other one: still three different colours.
        let moved = [(1usize, 0usize), (1, 1), (0, 0)];
        let mut ranks: Vec<usize> = moved.iter().map(|b| rank(moved, b)).collect();
        ranks.sort();
        assert_eq!(ranks, [0, 1, 2]);
    }

    #[test]
    fn first_body_keeps_the_classic_colour_and_ranks_wrap() {
        assert_eq!(default_color(0), [170, 180, 195]);
        assert_eq!(default_color(PALETTE.len()), default_color(0));
        assert_ne!(default_color(1), default_color(0));
    }
}
