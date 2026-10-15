//! Expressions and user parameters (Fusion's Change Parameters): anything
//! typed into a length or angle box can be arithmetic with units, functions
//! and parameter names — "width * 2 + 5", "10 mm + 1 in", "sqrt(area)",
//! "atan(rise / run)".
//!
//! Values carry what they are: a length (mm; to a power, so an area is
//! length²), an angle (degrees) or a plain number. A bare number takes the
//! box's unit when it meets a length ("10 + 5 mm": the 10 is in the design's
//! units), and is a plain number otherwise ("width * 2").

use crate::units::Unit;
use serde::{Deserialize, Serialize};

/// What a box (or a parameter) holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Length,
    Angle,
    Plain,
}

/// A parameter's unit, as Fusion's Change Parameters lists them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamUnit {
    #[default]
    Mm,
    Cm,
    M,
    In,
    Ft,
    Deg,
    /// No unit (a count, a ratio).
    None,
}

impl ParamUnit {
    pub const ALL: [ParamUnit; 7] = [ParamUnit::Mm, ParamUnit::Cm, ParamUnit::M, ParamUnit::In, ParamUnit::Ft, ParamUnit::Deg, ParamUnit::None];

    pub fn name(self) -> &'static str {
        match self.length() {
            Some(u) => u.name(),
            None if self == ParamUnit::Deg => "deg",
            None => "no units",
        }
    }

    pub fn length(self) -> Option<Unit> {
        Some(match self {
            ParamUnit::Mm => Unit::Mm,
            ParamUnit::Cm => Unit::Cm,
            ParamUnit::M => Unit::M,
            ParamUnit::In => Unit::In,
            ParamUnit::Ft => Unit::Ft,
            ParamUnit::Deg | ParamUnit::None => return None,
        })
    }

    pub fn kind(self) -> Kind {
        match self {
            ParamUnit::Deg => Kind::Angle,
            ParamUnit::None => Kind::Plain,
            _ => Kind::Length,
        }
    }

    /// A stored value (mm, degrees, plain) shown in this unit.
    pub fn show(self, v: f64) -> String {
        match self.length() {
            Some(u) => format!("{} {}", u.plain(v), u.name()),
            None if self == ParamUnit::Deg => format!("{:.2} deg", v),
            None => format!("{:.4}", v).trim_end_matches('0').trim_end_matches('.').to_string(),
        }
    }
}

/// A named value the design's dimensions and features can use.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UserParameter {
    pub name: String,
    pub unit: ParamUnit,
    pub expression: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

/// A value with what it is: `len` is the power of length (0: none, 1: a
/// length in mm, 2: an area), `angle` an angle in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Value {
    pub v: f64,
    pub len: i32,
    pub angle: bool,
}

impl Value {
    fn plain(v: f64) -> Self {
        Value { v, len: 0, angle: false }
    }
    fn is_plain(&self) -> bool {
        self.len == 0 && !self.angle
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    Op(char),
}

fn tokens(text: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                i += 1;
            }
            // An exponent: 1e3, 2.5E-4 (not "2 e" or a unit starting with e).
            if i < cs.len() && matches!(cs[i], 'e' | 'E') {
                let mut j = i + 1;
                if j < cs.len() && matches!(cs[j], '+' | '-') {
                    j += 1;
                }
                if j < cs.len() && cs[j].is_ascii_digit() {
                    while j < cs.len() && cs[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let s: String = cs[start..i].iter().collect();
            out.push(Tok::Num(s.parse().map_err(|_| format!("can't read the number {s}"))?));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            out.push(Tok::Name(cs[start..i].iter().collect()));
        } else if "+-*/^(),".contains(c) {
            out.push(Tok::Op(c));
            i += 1;
        } else if matches!(c, '"' | '\'' | '°') {
            // Inches, feet, degrees: a unit written as a mark.
            out.push(Tok::Name(c.to_string()));
            i += 1;
        } else {
            return Err(format!("can't read '{c}'"));
        }
    }
    Ok(out)
}

/// A unit word after a number: (mm per unit, is it an angle).
fn unit_word(w: &str) -> Option<Value> {
    if let Some(u) = Unit::from_word(w) {
        return Some(Value { v: u.mm(), len: 1, angle: false });
    }
    match w.to_ascii_lowercase().as_str() {
        "°" | "deg" | "degree" | "degrees" => Some(Value { v: 1.0, len: 0, angle: true }),
        "rad" | "radian" | "radians" => Some(Value { v: 1f64.to_degrees(), len: 0, angle: true }),
        _ => None,
    }
}

struct Parser<'a> {
    toks: Vec<Tok>,
    at: usize,
    unit: Unit,
    lookup: &'a dyn Fn(&str) -> Option<Result<Value, String>>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(&Tok::Op(c)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    /// Make two values addable: a bare number meeting a length is in the box's unit.
    fn match_up(&self, a: Value, b: Value) -> Result<(Value, Value), String> {
        let promote = |x: Value, to: &Value| if x.is_plain() && to.len == 1 && !to.angle { Value { v: x.v * self.unit.mm(), len: 1, angle: false } } else if x.is_plain() && to.angle && to.len == 0 { Value { angle: true, ..x } } else { x };
        let (a, b) = (promote(a, &b), promote(b, &a));
        if a.len != b.len || a.angle != b.angle {
            return Err("adds a length to something that isn't one".into());
        }
        Ok((a, b))
    }

    fn sum(&mut self) -> Result<Value, String> {
        let mut acc = self.product()?;
        loop {
            let plus = if self.eat('+') {
                true
            } else if self.eat('-') {
                false
            } else {
                return Ok(acc);
            };
            let rhs = self.product()?;
            let (a, b) = self.match_up(acc, rhs)?;
            acc = Value { v: if plus { a.v + b.v } else { a.v - b.v }, ..a };
        }
    }

    fn product(&mut self) -> Result<Value, String> {
        let mut acc = self.power()?;
        loop {
            let times = if self.eat('*') {
                true
            } else if self.eat('/') {
                false
            } else {
                return Ok(acc);
            };
            let rhs = self.power()?;
            acc = if times {
                Value { v: acc.v * rhs.v, len: acc.len + rhs.len, angle: acc.angle || rhs.angle }
            } else {
                if rhs.v == 0.0 {
                    return Err("divides by zero".into());
                }
                // An angle over an angle is a plain ratio.
                Value { v: acc.v / rhs.v, len: acc.len - rhs.len, angle: acc.angle && !rhs.angle }
            };
        }
    }

    fn power(&mut self) -> Result<Value, String> {
        let base = self.unary()?;
        if self.eat('^') {
            let e = self.power()?;
            if !e.is_plain() {
                return Err("a power must be a plain number".into());
            }
            let len = base.len as f64 * e.v;
            if len.fract() != 0.0 {
                return Err("that power leaves a fraction of a length".into());
            }
            return Ok(Value { v: base.v.powf(e.v), len: len as i32, angle: base.angle });
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<Value, String> {
        if self.eat('-') {
            let v = self.unary()?;
            return Ok(Value { v: -v.v, ..v });
        }
        if self.eat('+') {
            return self.unary();
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Value, String> {
        let t = self.peek().cloned().ok_or("ends too soon")?;
        self.at += 1;
        let v = match t {
            Tok::Num(n) => Value::plain(n),
            Tok::Op('(') => {
                let v = self.sum()?;
                if !self.eat(')') {
                    return Err("a ( isn't closed".into());
                }
                v
            }
            Tok::Name(name) if self.peek() == Some(&Tok::Op('(')) => {
                self.at += 1;
                let mut args = vec![self.sum()?];
                while self.eat(',') {
                    args.push(self.sum()?);
                }
                if !self.eat(')') {
                    return Err(format!("{name}( isn't closed"));
                }
                self.call(&name, args)?
            }
            Tok::Name(name) => match name.to_ascii_lowercase().as_str() {
                "pi" => Value::plain(std::f64::consts::PI),
                "e" => Value::plain(std::f64::consts::E),
                _ => (self.lookup)(&name).ok_or_else(|| format!("no parameter called {name}"))??,
            },
            Tok::Op(c) => return Err(format!("unexpected {c}")),
        };
        // A unit right after a plain value: "10 mm", "45°", "(1 + 2) in", "pi rad".
        if let (true, Some(Tok::Name(w))) = (v.is_plain(), self.peek()) {
            if let Some(u) = unit_word(w) {
                self.at += 1;
                return Ok(Value { v: v.v * u.v, ..u });
            }
        }
        Ok(v)
    }

    fn call(&self, name: &str, args: Vec<Value>) -> Result<Value, String> {
        let one = || -> Result<Value, String> {
            match args[..] {
                [a] => Ok(a),
                _ => Err(format!("{name} takes one value")),
            }
        };
        // Trig takes an angle (a bare number is degrees) and gives a plain number.
        let angle_in = |a: Value| -> Result<f64, String> {
            if a.len != 0 {
                return Err(format!("{name} takes an angle"));
            }
            Ok(a.v.to_radians())
        };
        let plain_in = |a: Value| -> Result<f64, String> { if a.is_plain() { Ok(a.v) } else { Err(format!("{name} takes a plain number")) } };
        let deg = |r: f64| Value { v: r.to_degrees(), len: 0, angle: true };
        Ok(match name.to_ascii_lowercase().as_str() {
            "sin" => Value::plain(angle_in(one()?)?.sin()),
            "cos" => Value::plain(angle_in(one()?)?.cos()),
            "tan" => Value::plain(angle_in(one()?)?.tan()),
            "asin" => deg(plain_in(one()?)?.asin()),
            "acos" => deg(plain_in(one()?)?.acos()),
            "atan" => deg(plain_in(one()?)?.atan()),
            "sqrt" => {
                let a = one()?;
                if a.len % 2 != 0 || a.angle {
                    return Err("sqrt of that leaves a fraction of a length".into());
                }
                if a.v < 0.0 {
                    return Err("sqrt of a negative number".into());
                }
                Value { v: a.v.sqrt(), len: a.len / 2, angle: false }
            }
            "abs" => {
                let a = one()?;
                Value { v: a.v.abs(), ..a }
            }
            "round" | "floor" | "ceil" => {
                let a = one()?;
                // In the box's unit, as Fusion rounds: round(10.4 mm) = 10 mm.
                let scale = if a.len == 1 { self.unit.mm() } else { 1.0 };
                let x = a.v / scale;
                let r = match name {
                    "round" => x.round(),
                    "floor" => x.floor(),
                    _ => x.ceil(),
                };
                Value { v: r * scale, ..a }
            }
            "min" | "max" => {
                let mut it = args.into_iter();
                let mut acc = it.next().ok_or(format!("{name} needs values"))?;
                for b in it {
                    let (x, y) = self.match_up(acc, b)?;
                    acc = Value { v: if name == "min" { x.v.min(y.v) } else { x.v.max(y.v) }, ..x };
                }
                acc
            }
            _ => return Err(format!("no function called {name}")),
        })
    }
}

/// Evaluate `text` as a `want` (a length in mm, an angle in degrees, or a
/// plain number). A bare number is in `unit` for a length, degrees for an
/// angle. `lookup` gives a parameter's value by name (None: no such one).
pub fn eval_with(text: &str, want: Kind, unit: Unit, lookup: &dyn Fn(&str) -> Option<Result<Value, String>>) -> Result<f64, String> {
    let toks = tokens(text)?;
    if toks.is_empty() {
        return Err("type a value".into());
    }
    let mut p = Parser { toks, at: 0, unit, lookup };
    let v = p.sum()?;
    if p.at < p.toks.len() {
        return Err("there's something extra at the end".into());
    }
    if !v.v.is_finite() {
        return Err("that isn't a number".into());
    }
    match want {
        Kind::Length if v.len == 1 && !v.angle => Ok(v.v),
        Kind::Length if v.is_plain() => Ok(v.v * unit.mm()),
        Kind::Length => Err("that isn't a length".into()),
        Kind::Angle if v.len == 0 => Ok(v.v),
        Kind::Angle => Err("that isn't an angle".into()),
        Kind::Plain if v.is_plain() => Ok(v.v),
        Kind::Plain => Err("that should have no unit".into()),
    }
}

fn as_value(v: f64, kind: Kind) -> Value {
    match kind {
        Kind::Length => Value { v, len: 1, angle: false },
        Kind::Angle => Value { v, len: 0, angle: true },
        Kind::Plain => Value::plain(v),
    }
}

/// Every parameter's value (mm, degrees or plain) or why it has none. They
/// may use each other in any order; a loop is an error.
pub fn evaluate(params: &[UserParameter]) -> Vec<Result<f64, String>> {
    use std::cell::RefCell;
    struct State {
        busy: Vec<usize>,
        done: Vec<Option<Result<f64, String>>>,
    }
    fn value_of(params: &[UserParameter], i: usize, st: &RefCell<State>) -> Result<f64, String> {
        if let Some(r) = st.borrow().done[i].clone() {
            return r;
        }
        if st.borrow().busy.contains(&i) {
            return Err(format!("{} uses itself (through other parameters)", params[i].name));
        }
        st.borrow_mut().busy.push(i);
        let p = &params[i];
        let lookup = |name: &str| {
            let j = params.iter().position(|q| q.name == name)?;
            Some(value_of(params, j, st).map(|v| as_value(v, params[j].unit.kind())))
        };
        let r = eval_with(&p.expression, p.unit.kind(), p.unit.length().unwrap_or(Unit::Mm), &lookup);
        st.borrow_mut().busy.pop();
        st.borrow_mut().done[i] = Some(r.clone());
        r
    }
    let st = RefCell::new(State { busy: Vec::new(), done: vec![None; params.len()] });
    (0..params.len()).map(|i| value_of(params, i, &st)).collect()
}

/// Is `name` a name a parameter can have (a letter or _, then letters,
/// digits, _), and not a function, constant or unit?
pub fn good_name(name: &str) -> bool {
    let mut cs = name.chars();
    cs.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && cs.all(|c| c.is_alphanumeric() || c == '_')
        && unit_word(name).is_none()
        && !["pi", "e", "sin", "cos", "tan", "asin", "acos", "atan", "sqrt", "abs", "round", "floor", "ceil", "min", "max"].contains(&name.to_ascii_lowercase().as_str())
}

/// `text` with the name `old` (as a whole name, not inside another) changed
/// to `new` — renaming a parameter keeps what uses it working.
pub fn rename(text: &str, old: &str, new: &str) -> String {
    let mut out = String::new();
    let cs: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        // A number (and the e of an exponent) is copied as it is.
        if c.is_ascii_digit() || (c == '.' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            while i < cs.len() && (cs[i].is_ascii_digit() || cs[i] == '.') {
                out.push(cs[i]);
                i += 1;
            }
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            let word: String = cs[start..i].iter().collect();
            out.push_str(if word == old { new } else { &word });
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The name being typed at the end of `text` and the parameter names it
/// could be (those holding its letters, the ones starting with them first;
/// at most `max`): (where the word starts, the names). None when the end
/// isn't a name being typed (a number, an operator, a whole name already).
pub fn suggestions(text: &str, names: &[String], max: usize) -> Option<(usize, Vec<String>)> {
    let start = text.char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_').last().map(|(i, _)| i)?;
    let word = &text[start..];
    // A unit after a number ("10 mm") or a number's exponent isn't a name.
    let before = text[..start].trim_end();
    if !word.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') || before.ends_with(|c: char| c.is_ascii_digit() || c == ')') {
        return None;
    }
    if names.iter().any(|n| n == word) {
        return None; // typed in full
    }
    let low = word.to_lowercase();
    let mut found: Vec<&String> = names.iter().filter(|n| n.to_lowercase().contains(&low) && *n != word).collect();
    found.sort_by_key(|n| (!n.to_lowercase().starts_with(&low), n.len()));
    let found: Vec<String> = found.into_iter().take(max).cloned().collect();
    (!found.is_empty()).then_some((start, found))
}

/// Does `text` use the name `name`?
pub fn uses(text: &str, name: &str) -> bool {
    tokens(text).is_ok_and(|t| t.iter().any(|k| *k == Tok::Name(name.to_string())))
}

thread_local! {
    /// The design's parameters, for the boxes that read typed values.
    static SCOPE: std::cell::RefCell<Vec<(String, Result<Value, String>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The design's parameters, for `eval` (set when they change).
pub fn set_scope(params: &[UserParameter]) {
    let values = evaluate(params);
    let scope = params.iter().zip(values).map(|(p, v)| (p.name.clone(), v.map(|v| as_value(v, p.unit.kind())))).collect();
    SCOPE.with(|s| *s.borrow_mut() = scope);
}

/// `text` as a `want`, with the design's parameters (see `set_scope`).
pub fn eval(text: &str, want: Kind, unit: Unit) -> Result<f64, String> {
    SCOPE.with(|s| {
        let scope = s.borrow();
        eval_with(text, want, unit, &|name| scope.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, unit: ParamUnit, expression: &str) -> UserParameter {
        UserParameter { name: name.into(), unit, expression: expression.into(), comment: String::new() }
    }

    fn len(text: &str, unit: Unit) -> Result<f64, String> {
        eval_with(text, Kind::Length, unit, &|_| None)
    }

    #[test]
    fn arithmetic_with_units() {
        assert_eq!(len("10 + 5", Unit::Mm), Ok(15.0));
        assert_eq!(len("2 * (3 + 4) - 1", Unit::Mm), Ok(13.0));
        assert_eq!(len("10 mm + 1 in", Unit::Mm), Ok(35.4));
        assert!((len("1 + 1 mm", Unit::In).unwrap() - 26.4).abs() < 1e-12, "the bare 1 is in the design's inches");
        assert_eq!(len("2^3", Unit::Mm), Ok(8.0));
        assert_eq!(len("-3 + 1e1", Unit::Mm), Ok(7.0));
        assert_eq!(len("0.5\"", Unit::Mm), Ok(12.7));
        assert_eq!(len("sqrt(9 mm * 4 mm)", Unit::Mm), Ok(6.0));
        assert_eq!(len("max(3, 4 mm)", Unit::Mm), Ok(4.0));
        assert_eq!(len("round(10.4)", Unit::Mm), Ok(10.0));
        assert!(len("90 deg", Unit::Mm).is_err(), "an angle isn't a length");
        assert!(len("2 mm * 3 mm", Unit::Mm).is_err(), "an area isn't a length");
        assert!(len("1 / 0", Unit::Mm).is_err());
        assert!(len("(1 + 2", Unit::Mm).is_err());
        assert!(len("", Unit::Mm).is_err());
        let ang = |t: &str| eval_with(t, Kind::Angle, Unit::Mm, &|_| None);
        assert!((ang("atan(1)").unwrap() - 45.0).abs() < 1e-12);
        assert!((ang("pi rad").unwrap() - 180.0).abs() < 1e-12);
        assert_eq!(ang("30 + 15°"), Ok(45.0));
        assert!((eval_with("sin(30)", Kind::Plain, Unit::Mm, &|_| None).unwrap() - 0.5).abs() < 1e-12, "trig in degrees");
    }

    #[test]
    fn parameters_use_each_other_in_any_order_and_loops_are_errors() {
        let params = vec![p("depth", ParamUnit::Mm, "width / 2"), p("width", ParamUnit::Mm, "40"), p("count", ParamUnit::None, "width / 10 mm"), p("tilt", ParamUnit::Deg, "atan(depth / width)")];
        let v = evaluate(&params);
        assert_eq!(v[0], Ok(20.0));
        assert_eq!(v[1], Ok(40.0));
        assert_eq!(v[2], Ok(4.0));
        assert!((v[3].clone().unwrap() - 26.565051177).abs() < 1e-6);
        // Inches: the parameter's bare numbers are in its own unit, stored in mm.
        assert_eq!(evaluate(&[p("t", ParamUnit::In, "2")]), vec![Ok(50.8)]);
        let looped = vec![p("a", ParamUnit::Mm, "b + 1"), p("b", ParamUnit::Mm, "a * 2"), p("c", ParamUnit::Mm, "5")];
        let v = evaluate(&looped);
        assert!(v[0].is_err() && v[1].is_err());
        assert_eq!(v[2], Ok(5.0));
        assert!(evaluate(&[p("x", ParamUnit::Mm, "nope + 1")])[0].as_ref().is_err_and(|e| e.contains("nope")));
    }

    #[test]
    fn typing_a_name_suggests_the_parameters_holding_it() {
        let names: Vec<String> = ["width", "depth", "wall", "lid_width", "w2"].iter().map(|s| s.to_string()).collect();
        let (at, found) = suggestions("span / wi", &names, 6).unwrap();
        assert_eq!(at, 7);
        assert_eq!(found, vec!["width", "lid_width"], "starting with it first");
        assert_eq!(suggestions("w", &names, 2).unwrap().1, vec!["w2", "wall"], "at most 2");
        assert!(suggestions("10 m", &names, 6).is_none(), "a unit after a number");
        assert!(suggestions("width", &names, 6).is_none(), "typed in full");
        assert!(suggestions("3 + ", &names, 6).is_none());
        assert!(suggestions("zz", &names, 6).is_none());
    }

    #[test]
    fn renaming_changes_whole_names_only() {
        assert_eq!(rename("width*2 + widths + w_width - 3e2", "width", "span"), "span*2 + widths + w_width - 3e2");
        assert!(uses("max(width, 3)", "width") && !uses("widths", "width"));
        assert!(good_name("plate_w2") && !good_name("2w") && !good_name("mm") && !good_name("sqrt") && !good_name("a b"));
    }

    #[test]
    fn boxes_read_the_designs_parameters() {
        set_scope(&[p("width", ParamUnit::Mm, "40"), p("angle", ParamUnit::Deg, "30")]);
        assert_eq!(eval("width * 2 + 5", Kind::Length, Unit::Mm), Ok(85.0));
        assert_eq!(eval("angle + 15", Kind::Angle, Unit::Mm), Ok(45.0));
        assert!(eval("height", Kind::Length, Unit::Mm).is_err());
        set_scope(&[]);
    }
}
