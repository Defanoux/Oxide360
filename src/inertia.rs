//! Carry-on scrolling for the ribbon: a quick sideways flick keeps the bar
//! gliding a little after the fingers lift, a slow drag stops where it is.

/// A flick must be at least this fast (points per second) to carry on.
pub const MIN_FLICK: f32 = 450.0;
/// How quickly the glide dies away (per second; higher = shorter glide).
const FRICTION: f32 = 5.0;
/// Even a hard flick doesn't go faster than this (points per second).
const MAX_SPEED: f32 = 2200.0;
/// What fraction of the flick's speed the glide starts with: "not too much".
const CARRY: f32 = 0.55;
/// Below this the glide is over.
const STOP: f32 = 40.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inertia {
    /// Recent scroll speed while the user is scrolling, then the glide speed.
    vel: f32,
    gliding: bool,
}

impl Inertia {
    /// One frame. `user_delta`: how far the bar was scrolled by the user this
    /// frame (points; 0 when they aren't scrolling); `dt` seconds.
    /// Returns how far to move the bar by itself this frame, if gliding.
    pub fn step(&mut self, user_delta: f32, dt: f32) -> Option<f32> {
        let dt = dt.clamp(1e-3, 0.1);
        if user_delta != 0.0 {
            // Follow the speed of the finger (smoothed over a few frames).
            let now = user_delta / dt;
            self.vel = if self.gliding || self.vel == 0.0 { now } else { 0.5 * self.vel + 0.5 * now };
            self.gliding = false;
            return None;
        }
        if !self.gliding {
            // The first frame without scrolling: carry on only after a real flick.
            if self.vel.abs() < MIN_FLICK {
                self.vel = 0.0;
                return None;
            }
            self.vel = (self.vel * CARRY).clamp(-MAX_SPEED, MAX_SPEED);
            self.gliding = true;
        }
        let move_by = self.vel * dt;
        self.vel *= (-FRICTION * dt).exp();
        if self.vel.abs() < STOP {
            *self = Self::default();
        }
        Some(move_by)
    }

    /// Stop gliding (the bar hit its end, or the user grabbed it).
    pub fn stop(&mut self) {
        *self = Self::default();
    }

    pub fn is_gliding(&self) -> bool {
        self.gliding
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total_glide(mut i: Inertia) -> f32 {
        let mut sum = 0.0;
        for _ in 0..600 {
            match i.step(0.0, 1.0 / 60.0) {
                Some(d) => sum += d,
                None => break,
            }
        }
        sum
    }

    /// Scroll at `speed` points/second for half a second, then let go.
    fn flick(speed: f32) -> Inertia {
        let mut i = Inertia::default();
        for _ in 0..30 {
            assert!(i.step(speed / 60.0, 1.0 / 60.0).is_none(), "no glide while the user is scrolling");
        }
        i
    }

    #[test]
    fn a_slow_drag_does_not_carry_on() {
        let mut i = flick(200.0);
        assert_eq!(i.step(0.0, 1.0 / 60.0), None);
        assert!(!i.is_gliding());
    }

    #[test]
    fn a_flick_glides_further_the_faster_it_was() {
        let (slow, fast) = (total_glide(flick(800.0)), total_glide(flick(1600.0)));
        assert!(slow > 20.0, "a medium flick carries on a little: {slow}");
        assert!(fast > slow * 1.6, "a faster flick goes further: {slow} vs {fast}");
        assert!(fast < 600.0, "but never far: {fast}");
    }

    #[test]
    fn the_glide_follows_the_flick_direction_and_dies_away() {
        let back = total_glide(flick(-1200.0));
        assert!(back < -20.0, "{back}");
        let mut i = flick(1200.0);
        let mut frames = 0;
        while i.step(0.0, 1.0 / 60.0).is_some() {
            frames += 1;
            assert!(frames < 600, "it must stop");
        }
        assert!(!i.is_gliding());
    }

    #[test]
    fn touching_again_stops_the_glide_and_speed_is_capped() {
        let mut i = flick(1500.0);
        assert!(i.step(0.0, 1.0 / 60.0).is_some());
        assert!(i.is_gliding());
        assert_eq!(i.step(3.0, 1.0 / 60.0), None);
        assert!(!i.is_gliding());
        // An absurd flick is capped.
        let mut wild = flick(100_000.0);
        let first = wild.step(0.0, 1.0 / 60.0).unwrap();
        assert!(first <= 2200.0 / 60.0 + 1e-3, "{first}");
    }
}
