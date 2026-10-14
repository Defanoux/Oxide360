//! Thread sizes for the Hole tool (and Thread later): ISO metric (coarse and
//! fine), Unified (UNC / UNF) and NPT pipe threads, with tap-drill (minor)
//! diameters, clearance holes and default counterbore / countersink sizes.
//! All lengths in millimetres.

/// Fusion's "Thread Type".
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum Standard {
    #[default]
    IsoMetric,
    Unified,
    /// Tapered pipe thread (1:16 on the diameter), for Taper Tapped holes.
    Npt,
}

impl Standard {
    pub const ALL: [Standard; 3] = [Standard::IsoMetric, Standard::Unified, Standard::Npt];

    pub fn name(self) -> &'static str {
        match self {
            Standard::IsoMetric => "ISO Metric profile",
            Standard::Unified => "ANSI Unified Screw Threads",
            Standard::Npt => "ANSI Taper Pipe Threads NPT",
        }
    }

    /// Thread classes (tolerance grades), the usual one first. Only a label:
    /// the geometry is the basic profile.
    pub fn classes(self, internal: bool) -> &'static [&'static str] {
        match (self, internal) {
            (Standard::IsoMetric, true) => &["6H", "5H", "7H"],
            (Standard::IsoMetric, false) => &["6g", "6h", "4g6g"],
            (Standard::Unified, true) => &["2B", "1B", "3B"],
            (Standard::Unified, false) => &["2A", "1A", "3A"],
            (Standard::Npt, _) => &["NPT"],
        }
    }

    /// Countersink angle for flat-head screws of this standard.
    pub fn countersink_angle(self) -> f64 {
        match self {
            Standard::Unified => 82.0,
            _ => 90.0,
        }
    }
}

/// One thread: e.g. size "M6", designation "M6x1".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thread {
    pub standard: Standard,
    /// Nominal size ("M6", "1/4", "#10", "1/8").
    pub size: &'static str,
    /// Size and pitch ("M6x1", "1/4-20 UNC", "1/8-27 NPT").
    pub designation: &'static str,
    /// Major (nominal) diameter; for NPT the pipe's outside diameter.
    pub major: f64,
    pub pitch: f64,
}

const IN: f64 = 25.4;

macro_rules! iso {
    ($size:literal, $des:literal, $d:expr, $p:expr) => {
        Thread { standard: Standard::IsoMetric, size: $size, designation: $des, major: $d, pitch: $p }
    };
}
macro_rules! unified {
    ($size:literal, $des:literal, $d:expr, $tpi:expr) => {
        Thread { standard: Standard::Unified, size: $size, designation: $des, major: $d * IN, pitch: IN / $tpi }
    };
}
macro_rules! npt {
    ($size:literal, $des:literal, $od:expr, $tpi:expr) => {
        Thread { standard: Standard::Npt, size: $size, designation: $des, major: $od * IN, pitch: IN / $tpi }
    };
}

pub const THREADS: &[Thread] = &[
    iso!("M1.6", "M1.6x0.35", 1.6, 0.35),
    iso!("M2", "M2x0.4", 2.0, 0.4),
    iso!("M2.5", "M2.5x0.45", 2.5, 0.45),
    iso!("M3", "M3x0.5", 3.0, 0.5),
    iso!("M4", "M4x0.7", 4.0, 0.7),
    iso!("M5", "M5x0.8", 5.0, 0.8),
    iso!("M6", "M6x1", 6.0, 1.0),
    iso!("M8", "M8x1.25", 8.0, 1.25),
    iso!("M8", "M8x1", 8.0, 1.0),
    iso!("M10", "M10x1.5", 10.0, 1.5),
    iso!("M10", "M10x1.25", 10.0, 1.25),
    iso!("M10", "M10x1", 10.0, 1.0),
    iso!("M12", "M12x1.75", 12.0, 1.75),
    iso!("M12", "M12x1.5", 12.0, 1.5),
    iso!("M12", "M12x1.25", 12.0, 1.25),
    iso!("M14", "M14x2", 14.0, 2.0),
    iso!("M14", "M14x1.5", 14.0, 1.5),
    iso!("M16", "M16x2", 16.0, 2.0),
    iso!("M16", "M16x1.5", 16.0, 1.5),
    iso!("M20", "M20x2.5", 20.0, 2.5),
    iso!("M20", "M20x1.5", 20.0, 1.5),
    iso!("M24", "M24x3", 24.0, 3.0),
    iso!("M24", "M24x2", 24.0, 2.0),
    iso!("M30", "M30x3.5", 30.0, 3.5),
    iso!("M30", "M30x2", 30.0, 2.0),
    unified!("#4", "#4-40 UNC", 0.112, 40.0),
    unified!("#4", "#4-48 UNF", 0.112, 48.0),
    unified!("#6", "#6-32 UNC", 0.138, 32.0),
    unified!("#6", "#6-40 UNF", 0.138, 40.0),
    unified!("#8", "#8-32 UNC", 0.164, 32.0),
    unified!("#8", "#8-36 UNF", 0.164, 36.0),
    unified!("#10", "#10-24 UNC", 0.190, 24.0),
    unified!("#10", "#10-32 UNF", 0.190, 32.0),
    unified!("1/4", "1/4-20 UNC", 0.25, 20.0),
    unified!("1/4", "1/4-28 UNF", 0.25, 28.0),
    unified!("5/16", "5/16-18 UNC", 0.3125, 18.0),
    unified!("5/16", "5/16-24 UNF", 0.3125, 24.0),
    unified!("3/8", "3/8-16 UNC", 0.375, 16.0),
    unified!("3/8", "3/8-24 UNF", 0.375, 24.0),
    unified!("7/16", "7/16-14 UNC", 0.4375, 14.0),
    unified!("7/16", "7/16-20 UNF", 0.4375, 20.0),
    unified!("1/2", "1/2-13 UNC", 0.5, 13.0),
    unified!("1/2", "1/2-20 UNF", 0.5, 20.0),
    unified!("5/8", "5/8-11 UNC", 0.625, 11.0),
    unified!("5/8", "5/8-18 UNF", 0.625, 18.0),
    unified!("3/4", "3/4-10 UNC", 0.75, 10.0),
    unified!("3/4", "3/4-16 UNF", 0.75, 16.0),
    unified!("1", "1-8 UNC", 1.0, 8.0),
    unified!("1", "1-12 UNF", 1.0, 12.0),
    npt!("1/16", "1/16-27 NPT", 0.3125, 27.0),
    npt!("1/8", "1/8-27 NPT", 0.405, 27.0),
    npt!("1/4", "1/4-18 NPT", 0.540, 18.0),
    npt!("3/8", "3/8-18 NPT", 0.675, 18.0),
    npt!("1/2", "1/2-14 NPT", 0.840, 14.0),
    npt!("3/4", "3/4-14 NPT", 1.050, 14.0),
    npt!("1", "1-11.5 NPT", 1.315, 11.5),
];

/// The basic 60° profile's fundamental triangle height per pitch.
pub const H_PER_PITCH: f64 = 0.866_025_403_784_438_6;
/// NPT: the diameter grows 1 in 16 along the axis.
pub const NPT_TAPER: f64 = 1.0 / 16.0;

/// How snug a clearance hole is (ISO 273 close / medium / coarse).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Fit {
    Close,
    #[default]
    Normal,
    Loose,
}

impl Fit {
    pub const ALL: [Fit; 3] = [Fit::Close, Fit::Normal, Fit::Loose];

    pub fn name(self) -> &'static str {
        match self {
            Fit::Close => "Close",
            Fit::Normal => "Normal",
            Fit::Loose => "Loose",
        }
    }
}

impl Thread {
    /// By designation ("M6x1").
    pub fn find(designation: &str) -> Option<Thread> {
        THREADS.iter().copied().find(|t| t.designation == designation)
    }

    /// The first (coarse) thread of a size.
    pub fn of_size(standard: Standard, size: &str) -> Option<Thread> {
        THREADS.iter().copied().find(|t| t.standard == standard && t.size == size)
    }

    /// Minor diameter of the internal thread (the tap drill, basic profile:
    /// D − 2 × 5/8 H). For NPT, at the small end.
    pub fn minor(&self) -> f64 {
        match self.standard {
            Standard::Npt => self.npt_small_pitch_diameter() - 0.8 * self.pitch,
            _ => self.major - 1.25 * H_PER_PITCH * self.pitch,
        }
    }

    /// NPT pitch diameter at the small end of the external thread
    /// (ASME B1.20.1: E0 = D − (0.05 D + 1.1) p, in inches).
    pub fn npt_small_pitch_diameter(&self) -> f64 {
        let (d, p) = (self.major / IN, self.pitch / IN);
        (d - (0.05 * d + 1.1) * p) * IN
    }

    /// Clearance hole for a screw of this size.
    pub fn clearance(&self, fit: Fit) -> f64 {
        let k = match fit {
            Fit::Close => 0,
            Fit::Normal => 1,
            Fit::Loose => 2,
        };
        // ISO 273 for metric; common drill sizes (close / free) for Unified.
        let table: &[(&str, [f64; 3])] = &[
            ("M1.6", [1.7, 1.8, 2.0]),
            ("M2", [2.2, 2.4, 2.6]),
            ("M2.5", [2.7, 2.9, 3.1]),
            ("M3", [3.2, 3.4, 3.6]),
            ("M4", [4.3, 4.5, 4.8]),
            ("M5", [5.3, 5.5, 5.8]),
            ("M6", [6.4, 6.6, 7.0]),
            ("M8", [8.4, 9.0, 10.0]),
            ("M10", [10.5, 11.0, 12.0]),
            ("M12", [13.0, 13.5, 14.5]),
            ("M14", [15.0, 15.5, 16.5]),
            ("M16", [17.0, 17.5, 18.5]),
            ("M20", [21.0, 22.0, 24.0]),
            ("M24", [25.0, 26.0, 28.0]),
            ("M30", [31.0, 33.0, 35.0]),
            ("#4", [0.116 * IN, 0.1285 * IN, 0.1405 * IN]),
            ("#6", [0.144 * IN, 0.1495 * IN, 0.1695 * IN]),
            ("#8", [0.1695 * IN, 0.177 * IN, 0.196 * IN]),
            ("#10", [0.196 * IN, 0.201 * IN, 0.221 * IN]),
            ("1/4", [0.257 * IN, 0.266 * IN, 0.281 * IN]),
            ("5/16", [0.323 * IN, 0.332 * IN, 0.344 * IN]),
            ("3/8", [0.386 * IN, 0.397 * IN, 0.406 * IN]),
            ("7/16", [0.453 * IN, 0.469 * IN, 0.484 * IN]),
            ("1/2", [0.516 * IN, 0.531 * IN, 0.562 * IN]),
            ("5/8", [0.641 * IN, 0.656 * IN, 0.688 * IN]),
            ("3/4", [0.766 * IN, 0.781 * IN, 0.812 * IN]),
            ("1", [1.016 * IN, 1.031 * IN, 1.062 * IN]),
        ];
        table
            .iter()
            .find(|(s, _)| *s == self.size)
            .map_or(self.major * [1.05, 1.1, 1.2][k], |(_, v)| v[k])
    }

    /// Counterbore for a socket head cap screw: (diameter, depth).
    pub fn counterbore(&self) -> (f64, f64) {
        let table: &[(&str, f64, f64)] = &[
            ("M1.6", 3.5, 1.8),
            ("M2", 4.4, 2.3),
            ("M2.5", 5.5, 2.8),
            ("M3", 6.5, 3.4),
            ("M4", 8.0, 4.4),
            ("M5", 10.0, 5.4),
            ("M6", 11.0, 6.4),
            ("M8", 15.0, 8.6),
            ("M10", 18.0, 10.6),
            ("M12", 20.0, 12.6),
            ("M14", 24.0, 14.6),
            ("M16", 26.0, 16.6),
            ("M20", 33.0, 20.6),
            ("M24", 40.0, 24.8),
            ("M30", 48.0, 31.0),
        ];
        match table.iter().find(|(s, ..)| *s == self.size) {
            Some(&(_, d, h)) => (d, h),
            // Unified socket heads are 1.5 D across and D tall: a little over.
            None => (self.major * 1.5 + 0.8, self.major + 0.4),
        }
    }

    /// Countersink diameter for a flat-head screw (angle: the standard's).
    pub fn countersink(&self) -> f64 {
        self.major * 2.0 + 0.4
    }
}

/// The sizes of a standard, each once, in table order.
pub fn sizes(standard: Standard) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for t in THREADS.iter().filter(|t| t.standard == standard) {
        if !out.contains(&t.size) {
            out.push(t.size);
        }
    }
    out
}

/// The designations (pitches) of one size.
pub fn designations(standard: Standard, size: &str) -> Vec<&'static str> {
    THREADS.iter().filter(|t| t.standard == standard && t.size == size).map(|t| t.designation).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_sizes_have_their_pitch_and_tap_drill() {
        let m6 = Thread::find("M6x1").unwrap();
        assert!((m6.minor() - 4.917).abs() < 1e-3, "{}", m6.minor());
        let m10 = Thread::of_size(Standard::IsoMetric, "M10").unwrap();
        assert_eq!(m10.designation, "M10x1.5", "coarse first");
        assert!((m10.minor() - 8.376).abs() < 1e-3);
        let q = Thread::find("1/4-20 UNC").unwrap();
        assert!((q.pitch - 1.27).abs() < 1e-12 && (q.major - 6.35).abs() < 1e-12);
        assert!((q.minor() / IN - 0.1959).abs() < 1e-3, "{}", q.minor() / IN);
        // 1/8 NPT: E0 = 0.36351 in.
        let n = Thread::find("1/8-27 NPT").unwrap();
        assert!((n.npt_small_pitch_diameter() / IN - 0.36351).abs() < 1e-4);
    }

    #[test]
    fn lists_clearances_and_heads() {
        assert_eq!(sizes(Standard::IsoMetric)[..4], ["M1.6", "M2", "M2.5", "M3"]);
        assert_eq!(designations(Standard::IsoMetric, "M12"), vec!["M12x1.75", "M12x1.5", "M12x1.25"]);
        assert!(designations(Standard::Unified, "1/4").contains(&"1/4-28 UNF"));
        let m6 = Thread::find("M6x1").unwrap();
        assert_eq!([Fit::Close, Fit::Normal, Fit::Loose].map(|f| m6.clearance(f)), [6.4, 6.6, 7.0]);
        assert_eq!(m6.counterbore(), (11.0, 6.4));
        for t in THREADS {
            let (cd, ch) = t.counterbore();
            assert!(cd > t.clearance(Fit::Loose) && ch > 0.0, "{} counterbore clears the screw", t.designation);
            assert!(t.minor() > 0.0 && t.minor() < t.major, "{}", t.designation);
            assert!(t.clearance(Fit::Close) > t.major * 0.99 || t.standard == Standard::Npt, "{}", t.designation);
        }
    }
}
