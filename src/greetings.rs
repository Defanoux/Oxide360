//! The little line on the start page under the name, by the time of day:
//! one picked at random each time the app opens.

/// The six parts of the day: (from hour, to hour, the lines).
pub const PERIODS: [(u32, u32, &[&str]); 6] = [
    (0, 5, LATE_NIGHT),
    (5, 8, EARLY_MORNING),
    (8, 12, MORNING),
    (12, 17, AFTERNOON),
    (17, 21, EVENING),
    (21, 24, NIGHT),
];

const LATE_NIGHT: &[&str] = &[
    "Working late?",
    "The best ideas show up after midnight.",
    "Still up? So are we.",
    "Burning the midnight oil.",
    "Night shift at the workbench.",
    "The world's asleep. The printer isn't.",
    "Quiet hours, clean geometry.",
    "One more feature, then bed. Probably.",
    "Insomnia, but make it parametric.",
    "Midnight modelling session engaged.",
    "The owls approve of your design.",
    "No meetings at this hour. Just you and the timeline.",
    "It's dark out. Your fillets are still smooth.",
    "Late-night tolerances: double-check them in the morning.",
    "The stars are out, and so are your sketches.",
    "Shh. The constraints are sleeping.",
    "Running on coffee and constraints.",
    "Save often. It's late.",
    "Who needs sleep when you have Extrude?",
    "A night owl's workshop.",
    "Small hours, big ideas.",
    "Nothing like a 2 a.m. breakthrough.",
    "The moon called. It wants a bracket.",
    "Dreaming in millimetres?",
    "Late night, steady hands.",
    "Every great part started as a late-night sketch.",
    "Still here? Let's make something.",
    "The printer hums. The house sleeps.",
    "Working while the world recharges.",
    "Night mode: fully operational.",
    "The quiet makes the geometry clearer.",
    "Remember to blink.",
    "After hours, before deadlines.",
    "Midnight is just another dimension.",
];

const EARLY_MORNING: &[&str] = &[
    "Rise and shine!",
    "Up with the sun.",
    "Early bird gets the clean geometry.",
    "Coffee first, constraints second.",
    "A fresh day, a fresh sketch.",
    "Good morning, early riser.",
    "The day hasn't started yet. You have.",
    "Dawn patrol at the drawing board.",
    "Sunrise and a blank timeline.",
    "Before the emails: design time.",
    "Morning light makes everything look solid.",
    "Quiet morning, sharp ideas.",
    "First coffee, first extrude.",
    "Beat the rush. Model something.",
    "The early hours are the productive ones.",
    "Good morning! What are we building?",
    "Up early? The printer's warming up.",
    "Fresh start, fresh origin.",
    "Stretch, sip, sketch.",
    "Waking up the workshop.",
    "Bright and early, square and level.",
    "The birds are up. So is your design.",
    "A calm start to a precise day.",
    "Morning, maker.",
    "Clear head, clean lines.",
    "Let's get a part done before breakfast.",
    "Sun's coming up. So are the walls.",
    "Early start, early finish?",
    "The first sketch of the day is the best one.",
    "Good morning. Your timeline missed you.",
    "Toast, tea and tolerances.",
    "Ready when you are.",
    "An early start pays off.",
];

const MORNING: &[&str] = &[
    "Good morning!",
    "Let's make something today.",
    "Morning! Ready to build?",
    "A good morning for a good design.",
    "What's on the workbench today?",
    "Fresh ideas, fresh coffee.",
    "Time to turn sketches into solids.",
    "Measure twice, extrude once.",
    "The morning is yours. Make it count.",
    "Good morning. All systems parametric.",
    "Let's get those dimensions locked in.",
    "A productive morning starts here.",
    "Morning! The timeline is waiting.",
    "Today's agenda: make cool stuff.",
    "Second coffee? Second sketch.",
    "Plenty of daylight left for fillets.",
    "Good morning. Your parts won't design themselves.",
    "Rolling up the sleeves.",
    "Ready, set, sketch.",
    "Sharp pencils, sharp edges. Then fillet them.",
    "A fine morning for a fine part.",
    "The workshop is open.",
    "Morning momentum: go.",
    "What will you make today?",
    "Hello! Let's get building.",
    "Good morning. Constrain everything.",
    "The best time to start a design is now.",
    "Morning brain, maximum precision.",
    "Let's make the printer earn its keep.",
    "Off to a good start.",
    "Every great assembly starts with one sketch.",
    "Good morning. Fully defined is the goal.",
    "Bright morning, bright ideas.",
    "Let's do this.",
];

const AFTERNOON: &[&str] = &[
    "Good afternoon!",
    "Afternoon! How's the design coming along?",
    "Post-lunch productivity, engaged.",
    "Halfway through the day, halfway through the timeline.",
    "Afternoon slump? Try a revolve.",
    "Keep the momentum going.",
    "Good afternoon. Time to fillet something.",
    "The afternoon shift begins.",
    "Lunch is done. Let's build.",
    "A great afternoon for 3-D printing.",
    "Steady progress, solid results.",
    "Afternoon! The parts are calling.",
    "Plenty of day left.",
    "Back at it.",
    "Good afternoon, designer.",
    "Afternoon light, clean edges.",
    "One feature at a time.",
    "Need a break? Your sketch will wait.",
    "Keep calm and extrude on.",
    "The afternoon is for iterating.",
    "Version two is always better.",
    "Time for a quick prototype.",
    "Make it, print it, test it.",
    "Good afternoon. Check those clearances.",
    "Turning coffee into geometry since lunch.",
    "Afternoon tea and assemblies.",
    "Let's finish what we started.",
    "Still sunny. Still designing.",
    "An afternoon well spent is a part well made.",
    "Good afternoon. Let's get it fully defined.",
    "Small tweaks, big improvements.",
    "Hey there! Ready to keep going?",
    "The home stretch of the work day.",
];

const EVENING: &[&str] = &[
    "Good evening!",
    "Evening! Time for a side project?",
    "Winding down, or just getting started?",
    "Evening workshop hours.",
    "The day job's done. Now the fun part.",
    "Good evening. What are we making?",
    "Dinner, then design.",
    "A relaxing evening of modelling.",
    "Sunset sketches.",
    "Evening! The printer's ready when you are.",
    "Hobby hours have begun.",
    "Golden hour, golden ratio.",
    "Unwind with some geometry.",
    "Good evening. Let's make something fun.",
    "The best projects happen after hours.",
    "Evening calm, steady progress.",
    "Time to tinker.",
    "Grab a snack and open a sketch.",
    "Evening! Your workbench awaits.",
    "One more print before bed?",
    "A cosy evening of CAD.",
    "Back for an evening session?",
    "The sun's setting. Your design is rising.",
    "Weeknight project time.",
    "Good evening, maker.",
    "Plenty of time for one more feature.",
    "Evening plans: extrude, fillet, repeat.",
    "Relax. Constrain. Create.",
    "The evening shift is the fun shift.",
    "Lamps on, ideas on.",
    "Let's build something you'll use tomorrow.",
    "Nice to see you this evening.",
    "Let's make tonight productive.",
];

const NIGHT: &[&str] = &[
    "Good night, or good morning?",
    "Working late?",
    "Night owl mode: activated.",
    "The quiet hours are the best hours.",
    "Late night idea? Let's sketch it.",
    "The house is quiet. Let's make something.",
    "One more feature before bed?",
    "Starry night, steady geometry.",
    "Night time is design time.",
    "Burning the evening oil.",
    "Late, but not too late to extrude.",
    "Night shift, reporting in.",
    "The printer can run overnight.",
    "Dark outside, bright ideas inside.",
    "Late night tinkering session.",
    "Save before you sleep.",
    "Bedtime can wait. Probably.",
    "Night time, clean lines.",
    "Can't sleep? Design something.",
    "Moonlight modelling.",
    "Quiet hours, focused work.",
    "Wrapping up the day with a sketch.",
    "Good evening, night owl.",
    "The best ideas don't keep office hours.",
    "Set the print, then go to bed.",
    "A peaceful night at the workbench.",
    "One last tweak.",
    "Working the night shift on your next part.",
    "Late hours, long timelines.",
    "Still going strong.",
    "Stay up, sketch on.",
    "Tomorrow's prints start tonight.",
    "Night lights and fillets.",
];

/// The lines for `hour` (0–23, local time).
pub fn for_hour(hour: u32) -> &'static [&'static str] {
    PERIODS.iter().find(|(from, to, _)| (*from..*to).contains(&(hour % 24))).map_or(MORNING, |p| p.2)
}

/// One line for `hour`, chosen by `seed` (any number; the app uses the time).
pub fn pick(hour: u32, seed: u64) -> &'static str {
    let lines = for_hour(hour);
    // (A quick mix, so seeds in a row don't give lines in a row.)
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 29;
    lines[(x % lines.len() as u64) as usize]
}

/// The local hour now (from the system's timezone, via `date`; UTC if that fails).
pub fn local_hour() -> u32 {
    let out = std::process::Command::new("date").arg("+%H").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output();
    out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok()).unwrap_or_else(|| {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        ((secs / 3600) % 24) as u32
    })
}

/// Today's line for the start page: picked once when the app opens.
pub fn now() -> &'static str {
    static PICKED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    PICKED.get_or_init(|| {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        pick(local_hour(), seed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_hundred_lines_over_six_parts_of_the_day() {
        let all: Vec<&str> = PERIODS.iter().flat_map(|p| p.2.iter().copied()).collect();
        assert_eq!(all.len(), 200);
        assert!(PERIODS.iter().all(|p| p.2.len() >= 30));
        // Every hour of the day has its part, in order, with no gaps.
        assert_eq!(PERIODS[0].0, 0);
        assert_eq!(PERIODS[5].1, 24);
        for w in PERIODS.windows(2) {
            assert_eq!(w[0].1, w[1].0);
        }
        // No repeats within a part, and only what the UI font can draw (no arrows, ticks or emoji).
        for (_, _, lines) in PERIODS {
            let mut seen = std::collections::HashSet::new();
            for l in lines {
                assert!(seen.insert(*l), "twice: {l}");
                assert!(l.chars().all(|c| c.is_ascii() || "·—×°…".contains(c)), "can't draw: {l}");
                assert!(l.len() <= 60, "too long for one line: {l}");
            }
        }
    }

    #[test]
    fn the_hour_picks_the_part_of_the_day() {
        assert!(for_hour(2).contains(&"Working late?"));
        assert!(for_hour(6).contains(&"Rise and shine!"));
        assert!(for_hour(9).contains(&"Good morning!"));
        assert!(for_hour(14).contains(&"Good afternoon!"));
        assert!(for_hour(19).contains(&"Good evening!"));
        assert!(for_hour(23).contains(&"Night owl mode: activated."));
        // Different seeds give different lines (most of the time).
        let picks: std::collections::HashSet<&str> = (0..50).map(|s| pick(10, s)).collect();
        assert!(picks.len() > 10);
    }
}
