//! OpenSCAD files read in the app: the language (variables, modules,
//! functions, `for` / `if` / `let`, list comprehensions, `$fn`…) run here
//! into a tree of shapes (`Geo`), which `scad_kernel` makes into exact
//! OpenCASCADE solids. Each top-level object becomes a body of its own
//! (named after the module that made it, coloured by `color()`).
//!
//! The file's top-level `name = value;` lines are its parameters (as
//! OpenSCAD's Customizer shows them): they can be given other values.

use glam::{DMat4, DVec2, DVec3, DVec4};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// ---------- values ----------

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Undef,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Rc<Vec<Value>>),
    /// start, step, end (end included).
    Range(f64, f64, f64),
}

impl Value {
    fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(v))
    }

    fn truthy(&self) -> bool {
        match self {
            Value::Undef => false,
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Range(..) => true,
        }
    }

    pub fn num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn items(&self) -> Option<Vec<Value>> {
        match self {
            Value::List(l) => Some(l.as_ref().clone()),
            Value::Range(a, s, b) => Some(range_values(*a, *s, *b).into_iter().map(Value::Num).collect()),
            Value::Str(s) => Some(s.chars().map(|c| Value::Str(c.to_string())).collect()),
            _ => None,
        }
    }

    fn vec3(&self) -> Option<DVec3> {
        let l = self.items()?;
        let n = |i: usize| l.get(i).and_then(Value::num);
        Some(DVec3::new(n(0)?, n(1)?, n(2).unwrap_or(0.0)))
    }

    fn vec2(&self) -> Option<DVec2> {
        let l = self.items()?;
        Some(DVec2::new(l.first()?.num()?, l.get(1)?.num()?))
    }

    /// As OpenSCAD's `str()` writes it.
    pub fn show(&self) -> String {
        match self {
            Value::Undef => "undef".into(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => fmt_num(*n),
            Value::Str(s) => s.clone(),
            Value::List(l) => format!("[{}]", l.iter().map(Value::quoted).collect::<Vec<_>>().join(", ")),
            Value::Range(a, s, b) => format!("[{} : {} : {}]", fmt_num(*a), fmt_num(*s), fmt_num(*b)),
        }
    }

    /// As it would be typed (strings quoted).
    pub fn quoted(&self) -> String {
        match self {
            Value::Str(s) => format!("{s:?}"),
            v => v.show(),
        }
    }
}

/// A number as OpenSCAD prints it: up to 6 significant digits, no trailing zeros.
pub fn fmt_num(n: f64) -> String {
    if n.is_nan() {
        return "nan".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if n == n.trunc() && n.abs() < 1e15 {
        return format!("{}", n as i64);
    }
    let digits = 6 - 1 - n.abs().log10().floor() as i32;
    let s = format!("{:.*}", digits.clamp(0, 15) as usize, n);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

fn range_values(a: f64, s: f64, b: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if s == 0.0 || !a.is_finite() || !b.is_finite() || (s > 0.0 && a > b) || (s < 0.0 && a < b) {
        return out;
    }
    let n = ((b - a) / s + 1e-9).floor() as i64;
    if n > 1_000_000 {
        return out;
    }
    for i in 0..=n {
        out.push(a + i as f64 * s);
    }
    out
}

// ---------- tokens ----------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Id(String),
    /// `include <…>` / `use <…>`: (is it include?, the file).
    Include(bool, String),
    P(&'static str),
    End,
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, String> {
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0usize, 1usize);
    const PUNCT: [&str; 29] = ["<=", ">=", "==", "!=", "&&", "||", "(", ")", "[", "]", "{", "}", ",", ";", "=", "?", ":", "+", "-", "*", "/", "%", "^", "!", "<", ">", ".", "#", "$"];
    while i < c.len() {
        let ch = c[i];
        if ch == '\n' {
            line += 1;
            i += 1;
        } else if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                if c[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                i += 1;
            }
            if i < c.len() && (c[i] == 'e' || c[i] == 'E') {
                let mut j = i + 1;
                if j < c.len() && (c[j] == '+' || c[j] == '-') {
                    j += 1;
                }
                if j < c.len() && c[j].is_ascii_digit() {
                    i = j;
                    while i < c.len() && c[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let s: String = c[start..i].iter().collect();
            out.push((Tok::Num(s.parse().map_err(|_| format!("line {line}: bad number {s}"))?), line));
        } else if ch == '"' {
            i += 1;
            let mut s = String::new();
            while i < c.len() && c[i] != '"' {
                if c[i] == '\\' && i + 1 < c.len() {
                    i += 1;
                    match c[i] {
                        'n' => s.push('\n'),
                        't' => s.push('\t'),
                        'r' => s.push('\r'),
                        'u' | 'x' => {
                            let len = if c[i] == 'u' { 4 } else { 2 };
                            let hex: String = c[i + 1..(i + 1 + len).min(c.len())].iter().collect();
                            if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                s.push(ch);
                                i += len;
                            }
                        }
                        other => s.push(other),
                    }
                } else {
                    if c[i] == '\n' {
                        line += 1;
                    }
                    s.push(c[i]);
                }
                i += 1;
            }
            i += 1;
            out.push((Tok::Str(s), line));
        } else if ch.is_alphabetic() || ch == '_' || (ch == '$' && c.get(i + 1).is_some_and(|d| d.is_alphanumeric() || *d == '_')) {
            let start = i;
            i += 1;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            let id: String = c[start..i].iter().collect();
            if id == "include" || id == "use" {
                let mut j = i;
                while j < c.len() && c[j].is_whitespace() {
                    j += 1;
                }
                if c.get(j) == Some(&'<') {
                    let end = c[j..].iter().position(|&x| x == '>').map(|p| p + j).ok_or(format!("line {line}: {id} < without >"))?;
                    out.push((Tok::Include(id == "include", c[j + 1..end].iter().collect()), line));
                    i = end + 1;
                    continue;
                }
            }
            out.push((Tok::Id(id), line));
        } else {
            let rest: String = c[i..(i + 2).min(c.len())].iter().collect();
            let p = PUNCT.iter().find(|p| rest.starts_with(**p)).ok_or(format!("line {line}: unexpected '{ch}'"))?;
            out.push((Tok::P(p), line));
            i += p.chars().count();
        }
    }
    out.push((Tok::End, line));
    Ok(out)
}

// ---------- syntax ----------

#[derive(Clone, Debug)]
enum Expr {
    Lit(Value),
    Var(String),
    List(Vec<Item>),
    Range(Box<Expr>, Option<Box<Expr>>, Box<Expr>),
    Un(&'static str, Box<Expr>),
    Bin(&'static str, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(String, Vec<Arg>),
    Index(Box<Expr>, Box<Expr>),
    Member(Box<Expr>, String),
    Let(Vec<Arg>, Box<Expr>),
}

/// An entry in a list: a value, or a comprehension.
#[derive(Clone, Debug)]
enum Item {
    E(Expr),
    For(Vec<Arg>, Box<Item>),
    If(Expr, Box<Item>, Option<Box<Item>>),
    Let(Vec<Arg>, Box<Item>),
    Each(Box<Item>),
}

#[derive(Clone, Debug)]
struct Arg {
    name: Option<String>,
    value: Expr,
}

#[derive(Clone, Debug)]
enum Stmt {
    Assign(String, Expr, usize),
    Module(String, Rc<Def>),
    Function(String, Rc<FuncDef>),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    Inst(Inst),
    Block(Vec<Stmt>),
    Include(bool, String),
}

#[derive(Clone, Debug)]
struct Inst {
    modifier: Option<char>,
    name: String,
    args: Vec<Arg>,
    children: Rc<Vec<Stmt>>,
    line: usize,
}

#[derive(Debug)]
struct Def {
    params: Vec<(String, Option<Expr>)>,
    body: Vec<Stmt>,
}

#[derive(Debug)]
struct FuncDef {
    params: Vec<(String, Option<Expr>)>,
    body: Expr,
}

struct Parser {
    t: Vec<(Tok, usize)>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.t[self.i].0
    }
    fn peek_at(&self, k: usize) -> &Tok {
        &self.t[(self.i + k).min(self.t.len() - 1)].0
    }
    fn line(&self) -> usize {
        self.t[self.i].1
    }
    fn next(&mut self) -> Tok {
        let t = self.t[self.i].0.clone();
        if self.i + 1 < self.t.len() {
            self.i += 1;
        }
        t
    }
    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::P(q) if *q == p)
    }
    fn is_id(&self, id: &str) -> bool {
        matches!(self.peek(), Tok::Id(q) if q == id)
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.next();
            true
        } else {
            false
        }
    }
    fn want(&mut self, p: &str) -> Result<(), String> {
        if self.eat(p) {
            Ok(())
        } else {
            Err(format!("line {}: expected '{p}', found {}", self.line(), self.what()))
        }
    }
    fn what(&self) -> String {
        match self.peek() {
            Tok::Num(n) => fmt_num(*n),
            Tok::Str(s) => format!("{s:?}"),
            Tok::Id(s) => s.clone(),
            Tok::Include(..) => "include".into(),
            Tok::P(p) => format!("'{p}'"),
            Tok::End => "the end of the file".into(),
        }
    }
    fn id(&mut self) -> Result<String, String> {
        match self.next() {
            Tok::Id(s) => Ok(s),
            _ => {
                self.i -= 1;
                Err(format!("line {}: expected a name, found {}", self.line(), self.what()))
            }
        }
    }

    fn stmts_until_end(&mut self) -> Result<Vec<Stmt>, String> {
        let mut out = Vec::new();
        while *self.peek() != Tok::End {
            out.push(self.stmt()?);
        }
        Ok(out)
    }

    fn block(&mut self) -> Result<Vec<Stmt>, String> {
        self.want("{")?;
        let mut out = Vec::new();
        while !self.eat("}") {
            if *self.peek() == Tok::End {
                return Err(format!("line {}: a '{{' is never closed", self.line()));
            }
            out.push(self.stmt()?);
        }
        Ok(out)
    }

    /// A statement as a list (what a child / branch is).
    fn body(&mut self) -> Result<Vec<Stmt>, String> {
        if self.eat(";") {
            return Ok(Vec::new());
        }
        if self.is("{") {
            return self.block();
        }
        Ok(vec![self.stmt()?])
    }

    fn params(&mut self) -> Result<Vec<(String, Option<Expr>)>, String> {
        self.want("(")?;
        let mut out = Vec::new();
        while !self.eat(")") {
            let name = self.id()?;
            let def = if self.eat("=") { Some(self.expr()?) } else { None };
            out.push((name, def));
            if !self.eat(",") {
                self.want(")")?;
                break;
            }
        }
        Ok(out)
    }

    fn args(&mut self) -> Result<Vec<Arg>, String> {
        self.want("(")?;
        let mut out = Vec::new();
        while !self.eat(")") {
            let named = matches!(self.peek(), Tok::Id(_)) && matches!(self.peek_at(1), Tok::P("="));
            let name = if named {
                let n = self.id()?;
                self.next();
                Some(n)
            } else {
                None
            };
            out.push(Arg { name, value: self.expr()? });
            if !self.eat(",") {
                self.want(")")?;
                break;
            }
        }
        Ok(out)
    }

    fn stmt(&mut self) -> Result<Stmt, String> {
        let line = self.line();
        match self.peek().clone() {
            Tok::P(";") => {
                self.next();
                Ok(Stmt::Block(Vec::new()))
            }
            Tok::P("{") => Ok(Stmt::Block(self.block()?)),
            Tok::Include(inc, f) => {
                self.next();
                Ok(Stmt::Include(inc, f))
            }
            Tok::Id(id) if id == "module" => {
                self.next();
                let name = self.id()?;
                let params = self.params()?;
                let body = self.body()?;
                Ok(Stmt::Module(name, Rc::new(Def { params, body })))
            }
            Tok::Id(id) if id == "function" => {
                self.next();
                let name = self.id()?;
                let params = self.params()?;
                self.want("=")?;
                let body = self.expr()?;
                self.want(";")?;
                Ok(Stmt::Function(name, Rc::new(FuncDef { params, body })))
            }
            Tok::Id(id) if id == "if" => {
                self.next();
                self.want("(")?;
                let c = self.expr()?;
                self.want(")")?;
                let then = self.body()?;
                let other = if self.is_id("else") {
                    self.next();
                    self.body()?
                } else {
                    Vec::new()
                };
                Ok(Stmt::If(c, then, other))
            }
            Tok::Id(_) if matches!(self.peek_at(1), Tok::P("=")) => {
                let name = self.id()?;
                self.next();
                let e = self.expr()?;
                self.want(";")?;
                Ok(Stmt::Assign(name, e, line))
            }
            Tok::P(m @ ("%" | "#" | "!" | "*")) => {
                self.next();
                let mut s = self.stmt()?;
                if let Stmt::Inst(inst) = &mut s {
                    inst.modifier = Some(m.chars().next().unwrap_or('#'));
                }
                Ok(s)
            }
            Tok::Id(_) => {
                let name = self.id()?;
                let args = self.args()?;
                let children = Rc::new(self.body()?);
                Ok(Stmt::Inst(Inst { modifier: None, name, args, children, line }))
            }
            _ => Err(format!("line {line}: unexpected {}", self.what())),
        }
    }

    fn expr(&mut self) -> Result<Expr, String> {
        if self.is_id("let") && matches!(self.peek_at(1), Tok::P("(")) {
            self.next();
            let a = self.args()?;
            return Ok(Expr::Let(a, Box::new(self.expr()?)));
        }
        if self.is_id("function") {
            return Err(format!("line {}: function literals aren't supported yet", self.line()));
        }
        let c = self.binary(0)?;
        if self.eat("?") {
            let a = self.expr()?;
            self.want(":")?;
            let b = self.expr()?;
            return Ok(Expr::If(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn binary(&mut self, level: usize) -> Result<Expr, String> {
        const LEVELS: [&[&str]; 6] = [&["||"], &["&&"], &["==", "!="], &["<", "<=", ">", ">="], &["+", "-"], &["*", "/", "%"]];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut a = self.binary(level + 1)?;
        loop {
            let op = match self.peek() {
                Tok::P(p) if LEVELS[level].contains(p) => *p,
                _ => break,
            };
            self.next();
            let b = self.binary(level + 1)?;
            a = Expr::Bin(op, Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        for op in ["!", "-", "+"] {
            if self.eat(op) {
                return Ok(Expr::Un(op, Box::new(self.unary()?)));
            }
        }
        let base = self.postfix()?;
        if self.eat("^") {
            return Ok(Expr::Bin("^", Box::new(base), Box::new(self.unary()?)));
        }
        Ok(base)
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = self.primary()?;
        loop {
            if self.is("(") {
                let Expr::Var(name) = e else {
                    return Err(format!("line {}: only named functions can be called", self.line()));
                };
                e = Expr::Call(name, self.args()?);
            } else if self.eat("[") {
                let i = self.expr()?;
                self.want("]")?;
                e = Expr::Index(Box::new(e), Box::new(i));
            } else if self.eat(".") {
                e = Expr::Member(Box::new(e), self.id()?);
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let line = self.line();
        match self.next() {
            Tok::Num(n) => Ok(Expr::Lit(Value::Num(n))),
            Tok::Str(s) => Ok(Expr::Lit(Value::Str(s))),
            Tok::Id(id) => Ok(match id.as_str() {
                "true" => Expr::Lit(Value::Bool(true)),
                "false" => Expr::Lit(Value::Bool(false)),
                "undef" => Expr::Lit(Value::Undef),
                _ => Expr::Var(id),
            }),
            Tok::P("(") => {
                let e = self.expr()?;
                self.want(")")?;
                Ok(e)
            }
            Tok::P("[") => {
                if self.eat("]") {
                    return Ok(Expr::List(Vec::new()));
                }
                let starts_item = |p: &Parser| p.is_id("for") || p.is_id("each") || (p.is_id("if") && matches!(p.peek_at(1), Tok::P("("))) || (p.is_id("let") && matches!(p.peek_at(1), Tok::P("(")));
                if !starts_item(self) {
                    let first = self.expr()?;
                    if self.eat(":") {
                        let second = self.expr()?;
                        let r = if self.eat(":") {
                            let third = self.expr()?;
                            Expr::Range(Box::new(first), Some(Box::new(second)), Box::new(third))
                        } else {
                            Expr::Range(Box::new(first), None, Box::new(second))
                        };
                        self.want("]")?;
                        return Ok(r);
                    }
                    let mut items = vec![Item::E(first)];
                    while self.eat(",") {
                        if self.is("]") {
                            break;
                        }
                        items.push(self.item()?);
                    }
                    self.want("]")?;
                    return Ok(Expr::List(items));
                }
                let mut items = vec![self.item()?];
                while self.eat(",") {
                    if self.is("]") {
                        break;
                    }
                    items.push(self.item()?);
                }
                self.want("]")?;
                Ok(Expr::List(items))
            }
            _ => {
                self.i -= 1;
                Err(format!("line {line}: unexpected {}", self.what()))
            }
        }
    }

    fn item(&mut self) -> Result<Item, String> {
        if self.is_id("for") {
            self.next();
            let a = self.args()?;
            return Ok(Item::For(a, Box::new(self.item()?)));
        }
        if self.is_id("each") {
            self.next();
            return Ok(Item::Each(Box::new(self.item()?)));
        }
        if self.is_id("let") && matches!(self.peek_at(1), Tok::P("(")) {
            self.next();
            let a = self.args()?;
            return Ok(Item::Let(a, Box::new(self.item()?)));
        }
        if self.is_id("if") && matches!(self.peek_at(1), Tok::P("(")) {
            self.next();
            self.want("(")?;
            let c = self.expr()?;
            self.want(")")?;
            let then = self.item()?;
            let other = if self.is_id("else") {
                self.next();
                Some(Box::new(self.item()?))
            } else {
                None
            };
            return Ok(Item::If(c, Box::new(then), other));
        }
        if self.eat("(") {
            // A parenthesised comprehension (or plain expression).
            let save = self.i;
            if self.is_id("for") || self.is_id("each") || self.is_id("if") || self.is_id("let") {
                let it = self.item()?;
                self.want(")")?;
                return Ok(it);
            }
            self.i = save - 1;
        }
        Ok(Item::E(self.expr()?))
    }
}

fn parse(src: &str) -> Result<Vec<Stmt>, String> {
    let mut p = Parser { t: lex(src)?, i: 0 };
    p.stmts_until_end()
}

// ---------- shapes ----------

/// A move applied to a shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Move {
    Translate(DVec3),
    /// About an axis through the origin, degrees.
    Rotate(DVec3, f64),
    Scale(DVec3),
    /// Reflected in the plane through the origin with this normal.
    Mirror(DVec3),
    Matrix(DMat4),
}

impl Move {
    pub fn matrix(&self) -> DMat4 {
        match self {
            Move::Translate(v) => DMat4::from_translation(*v),
            Move::Rotate(a, deg) => DMat4::from_axis_angle(a.normalize_or(DVec3::Z), deg.to_radians()),
            Move::Scale(s) => DMat4::from_scale(*s),
            Move::Mirror(n) => {
                let n = n.normalize_or(DVec3::X);
                let c = |v: DVec3| (v - 2.0 * n.dot(v) * n).extend(0.0);
                DMat4::from_cols(c(DVec3::X), c(DVec3::Y), c(DVec3::Z), DVec4::W)
            }
            Move::Matrix(m) => *m,
        }
    }
}

/// What a file draws. 2-D shapes lie on the XY plane.
#[derive(Clone, Debug, PartialEq)]
pub enum Geo {
    Cube { size: DVec3, center: bool },
    Sphere { r: f64 },
    /// `sides`: Some(n) when `$fn` asked for a few flat sides (a hex nut);
    /// None: truly round.
    Cylinder { h: f64, r1: f64, r2: f64, center: bool, sides: Option<usize> },
    Polyhedron { points: Vec<DVec3>, faces: Vec<Vec<usize>> },
    Square { size: DVec2, center: bool },
    Circle { r: f64, sides: Option<usize> },
    Polygon { points: Vec<DVec2>, paths: Vec<Vec<usize>> },
    Union(Vec<Geo>),
    Difference(Vec<Geo>),
    Intersection(Vec<Geo>),
    Hull(Vec<Geo>),
    Move(Move, Box<Geo>),
    LinearExtrude { height: f64, center: bool, child: Box<Geo> },
    RotateExtrude { angle: f64, child: Box<Geo> },
    /// Grown (shrunk when negative) by `amount`; round corners or sharp.
    Offset { amount: f64, round: bool, child: Box<Geo> },
    Color([f32; 4], Box<Geo>),
    /// A mesh or CAD file (`import()`), its path made absolute.
    Import(PathBuf),
}

/// Sides a round shape is drawn with when only an outline's points are
/// wanted (hulls of round things).
pub const ROUND_POINTS: usize = 72;

impl Geo {
    pub fn is_2d(&self) -> bool {
        match self {
            Geo::Square { .. } | Geo::Circle { .. } | Geo::Polygon { .. } => true,
            Geo::Union(c) | Geo::Difference(c) | Geo::Intersection(c) | Geo::Hull(c) => c.first().is_some_and(Geo::is_2d),
            Geo::Move(_, c) | Geo::Color(_, c) | Geo::Offset { child: c, .. } => c.is_2d(),
            _ => false,
        }
    }

    /// The points whose hull is this shape's hull (as moved by `m`); None
    /// when that takes the solid itself (a difference, an import).
    pub fn hull_points(&self, m: DMat4) -> Option<Vec<DVec3>> {
        let p = |v: DVec3| m.transform_point3(v);
        let ring = |r: f64, z: f64, n: usize| (0..n).map(move |k| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            DVec3::new(r * a.cos(), r * a.sin(), z)
        });
        Some(match self {
            Geo::Cube { size, center } => {
                let o = if *center { -*size / 2.0 } else { DVec3::ZERO };
                (0..8).map(|k| p(o + *size * DVec3::new((k & 1) as f64, ((k >> 1) & 1) as f64, ((k >> 2) & 1) as f64))).collect()
            }
            Geo::Sphere { r } => {
                let mut v = Vec::new();
                let n = ROUND_POINTS / 2;
                for i in 0..=n {
                    let lat = std::f64::consts::PI * (i as f64 / n as f64 - 0.5);
                    let ring_n = if i == 0 || i == n { 1 } else { ROUND_POINTS };
                    v.extend(ring(r * lat.cos(), r * lat.sin(), ring_n).map(p));
                }
                v
            }
            Geo::Cylinder { h, r1, r2, center, sides } => {
                let z0 = if *center { -h / 2.0 } else { 0.0 };
                let n = sides.unwrap_or(ROUND_POINTS);
                let mut v: Vec<DVec3> = ring(*r1, z0, if *r1 > 0.0 { n } else { 1 }).map(p).collect();
                v.extend(ring(*r2, z0 + h, if *r2 > 0.0 { n } else { 1 }).map(p));
                v
            }
            Geo::Polyhedron { points, .. } => points.iter().map(|q| p(*q)).collect(),
            Geo::Square { size, center } => {
                let o = if *center { -*size / 2.0 } else { DVec2::ZERO };
                [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].iter().map(|(a, b)| p((o + *size * DVec2::new(*a, *b)).extend(0.0))).collect()
            }
            Geo::Circle { r, sides } => ring(*r, 0.0, sides.unwrap_or(ROUND_POINTS)).map(p).collect(),
            Geo::Polygon { points, .. } => points.iter().map(|q| p(q.extend(0.0))).collect(),
            Geo::Union(c) | Geo::Hull(c) => {
                let mut v = Vec::new();
                for g in c {
                    v.extend(g.hull_points(m)?);
                }
                v
            }
            Geo::Move(mv, c) => c.hull_points(m * mv.matrix())?,
            Geo::Color(_, c) => c.hull_points(m)?,
            Geo::LinearExtrude { height, center, child } => {
                let z0 = if *center { -height / 2.0 } else { 0.0 };
                let mut v = child.hull_points(m * DMat4::from_translation(DVec3::Z * z0))?;
                v.extend(child.hull_points(m * DMat4::from_translation(DVec3::Z * (z0 + height)))?);
                v
            }
            _ => return None,
        })
    }
}

/// A top-level object of the file: one body.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// The module that made it, with its text arguments ("shell_half left").
    pub name: String,
    pub color: Option<[f32; 4]>,
    pub geo: Geo,
}

/// A top-level `name = value;` with a plain value: a parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub value: Value,
    /// The comment after it on its line (what it's for).
    pub note: String,
}

/// What running a file made.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub parts: Vec<Part>,
    /// `echo()` lines and warnings (what was left out, and why).
    pub messages: Vec<String>,
}

/// The file's parameters (top-level assignments of plain values), in order.
pub fn params(src: &str) -> Vec<Param> {
    let Ok(stmts) = parse(src) else { return Vec::new() };
    let lines: Vec<&str> = src.lines().collect();
    let mut out: Vec<Param> = Vec::new();
    for s in &stmts {
        let Stmt::Assign(name, e, line) = s else { continue };
        if name.starts_with('$') {
            continue;
        }
        let Some(value) = literal(e) else { continue };
        let note = lines.get(line.saturating_sub(1)).and_then(|l| l.split_once("//")).map_or(String::new(), |(_, c)| c.trim().to_string());
        out.retain(|p| p.name != *name);
        out.push(Param { name: name.clone(), value, note });
    }
    out
}

/// A plain value: numbers, strings, booleans, and lists of them.
fn literal(e: &Expr) -> Option<Value> {
    match e {
        Expr::Lit(v) => Some(v.clone()),
        Expr::Un("-", x) => literal(x).and_then(|v| v.num()).map(|n| Value::Num(-n)),
        Expr::List(items) => items.iter().map(|i| if let Item::E(e) = i { literal(e) } else { None }).collect::<Option<Vec<_>>>().map(Value::list),
        _ => None,
    }
}

/// Read a value typed for a parameter: a number, true / false, a list
/// ([1, 2]) or a string (quotes optional).
pub fn parse_value(text: &str) -> Value {
    let t = text.trim();
    if let Ok(stmts) = parse(&format!("x = {t};")) {
        if let Some(Stmt::Assign(_, e, _)) = stmts.first() {
            if let Some(v) = literal(e) {
                return v;
            }
        }
    }
    Value::Str(t.trim_matches('"').to_string())
}

/// Run OpenSCAD source `src` (from the file at `path`, for `include` /
/// `import`) with `overrides` for its parameters.
pub fn run(src: &str, path: Option<&Path>, overrides: &[(String, Value)]) -> Result<Output, String> {
    let stmts = parse(src)?;
    let mut ev = Eval { dir: path.and_then(Path::parent).map(Path::to_path_buf), messages: Vec::new(), depth: 0, root: None, overrides: overrides.to_vec(), steps: 0 };
    let scope = Scope::new(None);
    for (k, v) in [("$fn", 0.0), ("$fa", 12.0), ("$fs", 2.0), ("$t", 0.0), ("PI", std::f64::consts::PI)] {
        scope.set(k, Value::Num(v));
    }
    let stmts = ev.expand_includes(stmts, 0)?;
    let mut parts = Vec::new();
    ev.top(&stmts, &scope, &mut parts)?;
    if let Some(root) = ev.root.take() {
        parts = vec![root];
    }
    Ok(Output { parts, messages: ev.messages })
}

// ---------- running ----------

struct Scope {
    vars: RefCell<HashMap<String, Value>>,
    /// Modules and functions defined here, with where (names in them are looked up from there).
    modules: RefCell<HashMap<String, (Rc<Def>, std::rc::Weak<Scope>)>>,
    funcs: RefCell<HashMap<String, (Rc<FuncDef>, std::rc::Weak<Scope>)>>,
    /// Inside a module: the children it was given, and where they were written.
    children: RefCell<Option<(Rc<Vec<Stmt>>, Rc<Scope>)>>,
    parent: Option<Rc<Scope>>,
}

impl Scope {
    fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        Rc::new(Scope { vars: RefCell::default(), modules: RefCell::default(), funcs: RefCell::default(), children: RefCell::new(None), parent })
    }
    fn set(&self, k: &str, v: Value) {
        self.vars.borrow_mut().insert(k.to_string(), v);
    }
    fn get(&self, k: &str) -> Option<Value> {
        if let Some(v) = self.vars.borrow().get(k) {
            return Some(v.clone());
        }
        self.parent.as_ref()?.get(k)
    }
    fn module(&self, k: &str) -> Option<(Rc<Def>, std::rc::Weak<Scope>)> {
        if let Some(v) = self.modules.borrow().get(k) {
            return Some(v.clone());
        }
        self.parent.as_ref()?.module(k)
    }
    fn func(&self, k: &str) -> Option<(Rc<FuncDef>, std::rc::Weak<Scope>)> {
        if let Some(v) = self.funcs.borrow().get(k) {
            return Some(v.clone());
        }
        self.parent.as_ref()?.func(k)
    }
    fn kids(&self) -> Option<(Rc<Vec<Stmt>>, Rc<Scope>)> {
        if let Some(c) = self.children.borrow().as_ref() {
            return Some(c.clone());
        }
        self.parent.as_ref()?.kids()
    }
    /// Every `$` variable seen from here (they pass down to what's called).
    fn dollars(&self, out: &mut HashMap<String, Value>) {
        if let Some(p) = &self.parent {
            p.dollars(out);
        }
        for (k, v) in self.vars.borrow().iter().filter(|(k, _)| k.starts_with('$')) {
            out.insert(k.clone(), v.clone());
        }
    }
}

struct Eval {
    dir: Option<PathBuf>,
    messages: Vec<String>,
    depth: usize,
    root: Option<Part>,
    overrides: Vec<(String, Value)>,
    steps: u64,
}

/// Arguments given to a call: by position and by name.
struct Args {
    pos: Vec<Value>,
    named: Vec<(String, Value)>,
}

impl Args {
    fn get(&self, name: &str, pos: usize) -> Value {
        self.named.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.clone()).or_else(|| self.pos.get(pos).cloned()).unwrap_or(Value::Undef)
    }
    fn named(&self, name: &str) -> Value {
        self.named.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap_or(Value::Undef)
    }
}

const MAX_DEPTH: usize = 400;
const MAX_STEPS: u64 = 50_000_000;

impl Eval {
    fn expand_includes(&mut self, stmts: Vec<Stmt>, level: usize) -> Result<Vec<Stmt>, String> {
        let mut out = Vec::new();
        for s in stmts {
            match s {
                Stmt::Include(inc, f) => {
                    if level > 20 {
                        return Err("includes go too deep (a file including itself?)".into());
                    }
                    let path = self.dir.as_ref().map_or_else(|| PathBuf::from(&f), |d| d.join(&f));
                    let text = std::fs::read_to_string(&path).map_err(|e| format!("couldn't read {f} ({e})"))?;
                    let inner = self.expand_includes(parse(&text).map_err(|e| format!("{f}: {e}"))?, level + 1)?;
                    if inc {
                        out.extend(inner);
                    } else {
                        // `use`: only its modules and functions.
                        out.extend(inner.into_iter().filter(|s| matches!(s, Stmt::Module(..) | Stmt::Function(..))));
                    }
                }
                s => out.push(s),
            }
        }
        Ok(out)
    }

    fn tick(&mut self) -> Result<(), String> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return Err("it runs too long (an endless loop?)".into());
        }
        Ok(())
    }

    /// Definitions, then assignments (OpenSCAD sets a block's variables
    /// before anything in it is drawn).
    fn declare(&mut self, stmts: &[Stmt], scope: &Rc<Scope>, top: bool) -> Result<(), String> {
        for s in stmts {
            match s {
                Stmt::Module(n, d) => {
                    scope.modules.borrow_mut().insert(n.clone(), (d.clone(), Rc::downgrade(scope)));
                }
                Stmt::Function(n, f) => {
                    scope.funcs.borrow_mut().insert(n.clone(), (f.clone(), Rc::downgrade(scope)));
                }
                _ => {}
            }
        }
        for s in stmts {
            if let Stmt::Assign(n, e, _) = s {
                let v = match self.overrides.iter().find(|(k, _)| top && k == n) {
                    Some((_, v)) => v.clone(),
                    None => self.expr(e, scope)?,
                };
                scope.set(n, v);
            }
        }
        Ok(())
    }

    fn top(&mut self, stmts: &[Stmt], scope: &Rc<Scope>, parts: &mut Vec<Part>) -> Result<(), String> {
        self.declare(stmts, scope, scope.parent.is_none())?;
        for s in stmts {
            match s {
                Stmt::If(c, a, b) => {
                    let branch = if self.expr(c, scope)?.truthy() { a } else { b };
                    let inner = Scope::new(Some(scope.clone()));
                    self.top(branch, &inner, parts)?;
                }
                Stmt::Block(b) => {
                    let inner = Scope::new(Some(scope.clone()));
                    self.top(b, &inner, parts)?;
                }
                Stmt::Inst(i) if i.modifier == Some('%') || i.modifier == Some('*') => {}
                Stmt::Inst(i) => {
                    let geo = self.inst(i, scope)?;
                    let Some(geo) = geo else { continue };
                    let part = Part { name: self.part_name(i, scope), color: outer_color(&geo), geo };
                    if i.modifier == Some('!') {
                        self.root = Some(part.clone());
                    }
                    parts.push(part);
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A name for the object `i` makes: the first module of the file's own it
    /// calls, with its text arguments ("shell_half left"); else the built-in.
    fn part_name(&mut self, i: &Inst, scope: &Rc<Scope>) -> String {
        let mut at = i;
        loop {
            if scope.module(&at.name).is_some() {
                let texts: Vec<String> = at.args.iter().filter_map(|a| match self.expr(&a.value, scope) {
                    Ok(Value::Str(s)) => Some(s),
                    _ => None,
                }).collect();
                return std::iter::once(at.name.clone()).chain(texts).collect::<Vec<_>>().join(" ");
            }
            match at.children.as_slice() {
                [Stmt::Inst(c)] => at = c,
                _ => return at.name.clone(),
            }
        }
    }

    fn block(&mut self, stmts: &[Stmt], scope: &Rc<Scope>) -> Result<Vec<Geo>, String> {
        self.declare(stmts, scope, false)?;
        let mut out = Vec::new();
        for s in stmts {
            match s {
                Stmt::If(c, a, b) => {
                    let branch = if self.expr(c, scope)?.truthy() { a } else { b };
                    let inner = Scope::new(Some(scope.clone()));
                    out.extend(self.block(branch, &inner)?);
                }
                Stmt::Block(b) => {
                    let inner = Scope::new(Some(scope.clone()));
                    out.extend(self.block(b, &inner)?);
                }
                Stmt::Inst(i) => {
                    if i.modifier == Some('%') || i.modifier == Some('*') {
                        continue;
                    }
                    if let Some(g) = self.inst(i, scope)? {
                        if i.modifier == Some('!') {
                            self.root = Some(Part { name: i.name.clone(), color: outer_color(&g), geo: g.clone() });
                        }
                        out.push(g);
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    fn args(&mut self, args: &[Arg], scope: &Rc<Scope>) -> Result<Args, String> {
        let mut a = Args { pos: Vec::new(), named: Vec::new() };
        for x in args {
            let v = self.expr(&x.value, scope)?;
            match &x.name {
                Some(n) => a.named.push((n.clone(), v)),
                None => a.pos.push(v),
            }
        }
        Ok(a)
    }

    /// The children of a built-in, run in a scope with its `$` arguments.
    fn kids(&mut self, i: &Inst, scope: &Rc<Scope>, a: &Args) -> Result<Vec<Geo>, String> {
        let inner = Scope::new(Some(scope.clone()));
        for (n, v) in a.named.iter().filter(|(n, _)| n.starts_with('$')) {
            inner.set(n, v.clone());
        }
        self.block(&i.children, &inner)
    }

    /// `$fn` / `$fa` / `$fs` for a round shape of radius `r`: Some(n) for a
    /// shape with few flat sides on purpose (`$fn` up to 12), None for round.
    fn sides(&self, a: &Args, scope: &Rc<Scope>) -> Option<usize> {
        let fnv = a.named("$fn").num().or_else(|| scope.get("$fn").and_then(|v| v.num())).unwrap_or(0.0);
        (fnv >= 3.0 && fnv <= 12.0).then_some(fnv as usize)
    }

    fn inst(&mut self, i: &Inst, scope: &Rc<Scope>) -> Result<Option<Geo>, String> {
        self.tick()?;
        let line = i.line;
        let a = self.args(&i.args, scope)?;
        let num = |v: Value, d: f64| v.num().unwrap_or(d);
        let flag = |v: Value| v.truthy();
        let group = |mut v: Vec<Geo>| -> Option<Geo> {
            match v.len() {
                0 => None,
                1 => v.pop(),
                _ => Some(Geo::Union(v)),
            }
        };
        let moved = |m: Move, kids: Vec<Geo>| group(kids).map(|g| Geo::Move(m, Box::new(g)));
        Ok(match i.name.as_str() {
            "cube" => {
                let s = a.get("size", 0);
                let size = s.vec3().or_else(|| s.num().map(DVec3::splat)).unwrap_or(DVec3::ONE);
                (size.min_element() > 0.0).then_some(Geo::Cube { size, center: flag(a.get("center", 1)) })
            }
            "sphere" => {
                let r = a.get("r", 0).num().or_else(|| a.named("d").num().map(|d| d / 2.0)).unwrap_or(1.0);
                (r > 0.0).then_some(Geo::Sphere { r })
            }
            "cylinder" => {
                let h = num(a.get("h", 0), 1.0);
                // (By position: h, r1, r2, center.)
                let r = a.named("r").num().or_else(|| a.named("d").num().map(|d| d / 2.0));
                let r1 = a.get("r1", 1).num().or_else(|| a.named("d1").num().map(|d| d / 2.0)).or(r).unwrap_or(1.0);
                let r2 = a.get("r2", 2).num().or_else(|| a.named("d2").num().map(|d| d / 2.0)).or(r).unwrap_or(1.0);
                let center = flag(a.get("center", 3));
                (h > 0.0 && (r1 > 0.0 || r2 > 0.0)).then(|| Geo::Cylinder { h, r1: r1.max(0.0), r2: r2.max(0.0), center, sides: self.sides(&a, scope) })
            }
            "polyhedron" => {
                let points: Vec<DVec3> = a.get("points", 0).items().unwrap_or_default().iter().filter_map(Value::vec3).collect();
                let f = a.get("faces", 1);
                let f = if f == Value::Undef { a.named("triangles") } else { f };
                let faces: Vec<Vec<usize>> = f.items().unwrap_or_default().iter().map(|f| f.items().unwrap_or_default().iter().filter_map(|i| i.num().map(|n| n as usize)).collect()).collect();
                if faces.iter().flatten().any(|&k| k >= points.len()) {
                    return Err(format!("line {line}: polyhedron face uses a point that isn't there"));
                }
                Some(Geo::Polyhedron { points, faces })
            }
            "square" => {
                let s = a.get("size", 0);
                let size = s.vec2().or_else(|| s.num().map(DVec2::splat)).unwrap_or(DVec2::ONE);
                (size.min_element() > 0.0).then_some(Geo::Square { size, center: flag(a.get("center", 1)) })
            }
            "circle" => {
                let r = a.get("r", 0).num().or_else(|| a.named("d").num().map(|d| d / 2.0)).unwrap_or(1.0);
                (r > 0.0).then(|| Geo::Circle { r, sides: self.sides(&a, scope) })
            }
            "polygon" => {
                let points: Vec<DVec2> = a.get("points", 0).items().unwrap_or_default().iter().filter_map(Value::vec2).collect();
                let paths: Vec<Vec<usize>> = a.get("paths", 1).items().unwrap_or_default().iter().map(|p| p.items().unwrap_or_default().iter().filter_map(|i| i.num().map(|n| n as usize)).filter(|&k| k < points.len()).collect()).collect();
                (points.len() >= 3).then_some(Geo::Polygon { points, paths })
            }
            "translate" => {
                let v = a.get("v", 0).vec3().unwrap_or(DVec3::ZERO);
                moved(Move::Translate(v), self.kids(i, scope, &a)?)
            }
            "rotate" => {
                let kids = self.kids(i, scope, &a)?;
                let angle = a.get("a", 0);
                let axis = a.get("v", 1).vec3();
                let mut g = group(kids);
                if let Some(deg) = angle.num() {
                    g = g.map(|g| Geo::Move(Move::Rotate(axis.unwrap_or(DVec3::Z), deg), Box::new(g)));
                } else if let Some(r) = angle.vec3() {
                    // About X, then Y, then Z.
                    for (ax, deg) in [(DVec3::X, r.x), (DVec3::Y, r.y), (DVec3::Z, r.z)] {
                        if deg != 0.0 {
                            g = g.map(|g| Geo::Move(Move::Rotate(ax, deg), Box::new(g)));
                        }
                    }
                }
                g
            }
            "scale" => {
                let s = a.get("v", 0);
                let v = s.vec3().map(|v| if s.items().is_some_and(|l| l.len() < 3) { DVec3::new(v.x, v.y, 1.0) } else { v }).or_else(|| s.num().map(DVec3::splat)).unwrap_or(DVec3::ONE);
                moved(Move::Scale(v), self.kids(i, scope, &a)?)
            }
            "mirror" => {
                let v = a.get("v", 0).vec3().unwrap_or(DVec3::X);
                moved(Move::Mirror(v), self.kids(i, scope, &a)?)
            }
            "multmatrix" => {
                let rows = a.get("m", 0).items().unwrap_or_default();
                let mut m = DMat4::IDENTITY;
                for (r, row) in rows.iter().take(4).enumerate() {
                    for (c, v) in row.items().unwrap_or_default().iter().take(4).enumerate() {
                        let col = m.col_mut(c);
                        col[r] = v.num().unwrap_or(col[r]);
                    }
                }
                moved(Move::Matrix(m), self.kids(i, scope, &a)?)
            }
            "color" => {
                let kids = self.kids(i, scope, &a)?;
                let alpha = a.get("alpha", 1).num().unwrap_or(1.0) as f32;
                let c = a.get("c", 0);
                let rgba = match &c {
                    Value::Str(s) => named_color(s).map(|[r, g, b, a0]| [r, g, b, a0.min(alpha)]),
                    v => v.items().map(|l| {
                        let n = |k: usize, d: f32| l.get(k).and_then(Value::num).map_or(d, |x| x as f32);
                        [n(0, 1.0), n(1, 1.0), n(2, 1.0), n(3, alpha)]
                    }),
                };
                match rgba {
                    Some(rgba) => group(kids).map(|g| Geo::Color(rgba, Box::new(g))),
                    None => group(kids),
                }
            }
            "union" | "group" | "render" => group(self.kids(i, scope, &a)?),
            "difference" => {
                let kids = self.kids(i, scope, &a)?;
                if kids.len() > 1 { Some(Geo::Difference(kids)) } else { group(kids) }
            }
            "intersection" => {
                let kids = self.kids(i, scope, &a)?;
                if kids.len() > 1 { Some(Geo::Intersection(kids)) } else { group(kids) }
            }
            "hull" => {
                let kids = self.kids(i, scope, &a)?;
                (!kids.is_empty()).then_some(Geo::Hull(kids))
            }
            "minkowski" => {
                self.messages.push(format!("line {line}: minkowski() isn't supported yet — its first shape is used as it is"));
                self.kids(i, scope, &a)?.into_iter().next()
            }
            "linear_extrude" => {
                let height = a.get("height", 0).num().or_else(|| a.named("h").num()).unwrap_or(100.0);
                if a.named("twist").num().is_some_and(|t| t != 0.0) || a.named("scale").items().is_some_and(|s| s.iter().any(|v| v.num() != Some(1.0))) || a.named("scale").num().is_some_and(|s| s != 1.0) {
                    self.messages.push(format!("line {line}: linear_extrude's twist and scale aren't supported yet — extruded straight"));
                }
                let kids = self.kids(i, scope, &a)?;
                group(kids).filter(|_| height > 0.0).map(|g| Geo::LinearExtrude { height, center: flag(a.get("center", 1)), child: Box::new(g) })
            }
            "rotate_extrude" => {
                let angle = a.named("angle").num().unwrap_or(360.0).clamp(-360.0, 360.0);
                group(self.kids(i, scope, &a)?).map(|g| Geo::RotateExtrude { angle, child: Box::new(g) })
            }
            "offset" => {
                let kids = self.kids(i, scope, &a)?;
                let (amount, round) = match a.get("r", 0).num() {
                    Some(r) => (r, true),
                    None => (a.named("delta").num().unwrap_or(1.0), false),
                };
                group(kids).map(|g| Geo::Offset { amount, round, child: Box::new(g) })
            }
            "import" => {
                let f = match a.get("file", 0) {
                    Value::Str(s) => s,
                    _ => return Err(format!("line {line}: import() needs a file name")),
                };
                let p = self.dir.as_ref().map_or_else(|| PathBuf::from(&f), |d| d.join(&f));
                Some(Geo::Import(p))
            }
            "for" | "intersection_for" => {
                let mut out = Vec::new();
                self.each(&i.args, 0, scope, &mut |ev, s| {
                    out.extend(ev.block(&i.children, s)?);
                    Ok(())
                })?;
                if i.name == "intersection_for" && out.len() > 1 { Some(Geo::Intersection(out)) } else { group(out) }
            }
            "let" => {
                let inner = Scope::new(Some(scope.clone()));
                for x in &i.args {
                    let v = self.expr(&x.value, &inner)?;
                    inner.set(x.name.as_deref().unwrap_or("_"), v);
                }
                group(self.block(&i.children, &inner)?)
            }
            "echo" => {
                let parts: Vec<String> = a.pos.iter().map(Value::quoted).chain(a.named.iter().map(|(n, v)| format!("{n} = {}", v.quoted()))).collect();
                self.messages.push(format!("ECHO: {}", parts.join(", ")));
                group(self.kids(i, scope, &a)?)
            }
            "assert" => {
                if !a.get("condition", 0).truthy() {
                    let msg = a.get("message", 1);
                    return Err(format!("line {line}: assert failed{}", if msg == Value::Undef { String::new() } else { format!(": {}", msg.show()) }));
                }
                group(self.kids(i, scope, &a)?)
            }
            "children" => {
                let Some((kids, at)) = scope.kids() else { return Ok(None) };
                let pick = a.get("index", 0);
                // Each child is one statement; run the ones asked for in the scope they were written in.
                let inner = Scope::new(Some(at.clone()));
                let mut ds = HashMap::new();
                scope.dollars(&mut ds);
                for (k, v) in ds {
                    inner.set(&k, v);
                }
                self.declare(&kids, &inner, false)?;
                let wanted: Vec<usize> = match &pick {
                    Value::Undef => (0..kids.len()).collect(),
                    v => match v.num() {
                        Some(n) => vec![n as usize],
                        None => v.items().unwrap_or_default().iter().filter_map(Value::num).map(|n| n as usize).collect(),
                    },
                };
                let geo_kids: Vec<&Stmt> = kids.iter().filter(|s| matches!(s, Stmt::Inst(_) | Stmt::If(..) | Stmt::Block(_))).collect();
                let mut out = Vec::new();
                for k in wanted {
                    if let Some(s) = geo_kids.get(k) {
                        out.extend(self.block(std::slice::from_ref(*s), &inner)?);
                    }
                }
                group(out)
            }
            "text" | "projection" | "surface" | "roof" => {
                self.messages.push(format!("line {line}: {}() isn't supported yet — left out", i.name));
                None
            }
            name => {
                let Some((def, home)) = scope.module(name) else {
                    self.messages.push(format!("line {line}: there's no module called {name} — left out"));
                    return Ok(None);
                };
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(format!("line {line}: {name} calls itself too deeply"));
                }
                // Names in it come from where it was written; $ variables pass down from the caller.
                let inner = Scope::new(Some(home.upgrade().unwrap_or_else(|| scope.clone())));
                let mut ds = HashMap::new();
                scope.dollars(&mut ds);
                for (k, v) in ds {
                    inner.set(&k, v);
                }
                self.bind(&def.params, &a, &inner)?;
                inner.set("$children", Value::Num(i.children.len() as f64));
                *inner.children.borrow_mut() = Some((i.children.clone(), scope.clone()));
                let body = Scope::new(Some(inner));
                let r = self.block(&def.body, &body);
                self.depth -= 1;
                group(r?)
            }
        })
    }

    fn bind(&mut self, params: &[(String, Option<Expr>)], a: &Args, inner: &Rc<Scope>) -> Result<(), String> {
        for (k, (name, def)) in params.iter().enumerate() {
            let v = match a.named.iter().rev().find(|(n, _)| n == name) {
                Some((_, v)) => v.clone(),
                None => match a.pos.get(k) {
                    Some(v) => v.clone(),
                    None => match def {
                        Some(e) => self.expr(e, inner)?,
                        None => Value::Undef,
                    },
                },
            };
            inner.set(name, v);
        }
        for (n, v) in a.named.iter().filter(|(n, _)| n.starts_with('$')) {
            inner.set(n, v.clone());
        }
        Ok(())
    }

    /// Run `f` once per combination of the `for` variables in `args`.
    fn each(&mut self, args: &[Arg], k: usize, scope: &Rc<Scope>, f: &mut dyn FnMut(&mut Eval, &Rc<Scope>) -> Result<(), String>) -> Result<(), String> {
        let Some(arg) = args.get(k) else { return f(self, scope) };
        let v = self.expr(&arg.value, scope)?;
        let items = match v {
            Value::Range(a, s, b) => range_values(a, s, b).into_iter().map(Value::Num).collect(),
            Value::Undef => Vec::new(),
            other => other.items().unwrap_or_else(|| vec![other.clone()]),
        };
        for it in items {
            self.tick()?;
            let inner = Scope::new(Some(scope.clone()));
            inner.set(arg.name.as_deref().unwrap_or("_"), it);
            self.each(args, k + 1, &inner, f)?;
        }
        Ok(())
    }

    fn expr(&mut self, e: &Expr, scope: &Rc<Scope>) -> Result<Value, String> {
        self.tick()?;
        Ok(match e {
            Expr::Lit(v) => v.clone(),
            Expr::Var(n) => scope.get(n).unwrap_or(Value::Undef),
            Expr::List(items) => {
                let mut out = Vec::new();
                for it in items {
                    self.item(it, scope, &mut out)?;
                }
                Value::list(out)
            }
            Expr::Range(a, s, b) => {
                let a = self.expr(a, scope)?.num().unwrap_or(f64::NAN);
                let b = self.expr(b, scope)?.num().unwrap_or(f64::NAN);
                match s {
                    Some(s) => Value::Range(a, self.expr(s, scope)?.num().unwrap_or(1.0), b),
                    None if a > b => Value::Range(b, 1.0, a),
                    None => Value::Range(a, 1.0, b),
                }
            }
            Expr::Un(op, x) => {
                let v = self.expr(x, scope)?;
                match *op {
                    "!" => Value::Bool(!v.truthy()),
                    "-" => neg(&v),
                    _ => v,
                }
            }
            Expr::Bin("&&", a, b) => Value::Bool(self.expr(a, scope)?.truthy() && self.expr(b, scope)?.truthy()),
            Expr::Bin("||", a, b) => Value::Bool(self.expr(a, scope)?.truthy() || self.expr(b, scope)?.truthy()),
            Expr::Bin(op, a, b) => {
                let (a, b) = (self.expr(a, scope)?, self.expr(b, scope)?);
                binop(op, &a, &b)
            }
            Expr::If(c, a, b) => {
                if self.expr(c, scope)?.truthy() {
                    self.expr(a, scope)?
                } else {
                    self.expr(b, scope)?
                }
            }
            Expr::Index(x, i) => {
                let (x, i) = (self.expr(x, scope)?, self.expr(i, scope)?);
                match (x, i.num()) {
                    (Value::List(l), Some(k)) if k >= 0.0 => l.get(k as usize).cloned().unwrap_or(Value::Undef),
                    (Value::Str(s), Some(k)) if k >= 0.0 => s.chars().nth(k as usize).map_or(Value::Undef, |c| Value::Str(c.to_string())),
                    (r @ Value::Range(..), Some(k)) if k >= 0.0 => r.items().and_then(|l| l.get(k as usize).cloned()).unwrap_or(Value::Undef),
                    _ => Value::Undef,
                }
            }
            Expr::Member(x, m) => {
                let x = self.expr(x, scope)?;
                let k = match m.as_str() {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    _ => return Ok(Value::Undef),
                };
                match x {
                    Value::List(l) => l.get(k).cloned().unwrap_or(Value::Undef),
                    _ => Value::Undef,
                }
            }
            Expr::Let(args, body) => {
                let inner = Scope::new(Some(scope.clone()));
                for x in args {
                    let v = self.expr(&x.value, &inner)?;
                    inner.set(x.name.as_deref().unwrap_or("_"), v);
                }
                self.expr(body, &inner)?
            }
            Expr::Call(name, args) => {
                let a = self.args(args, scope)?;
                match scope.func(name) {
                    Some((f, home)) => {
                        self.depth += 1;
                        if self.depth > MAX_DEPTH * 10 {
                            return Err(format!("{name}() calls itself too deeply"));
                        }
                        let inner = Scope::new(Some(home.upgrade().unwrap_or_else(|| scope.clone())));
                        let mut ds = HashMap::new();
                        scope.dollars(&mut ds);
                        for (k, v) in ds {
                            inner.set(&k, v);
                        }
                        self.bind(&f.params, &a, &inner)?;
                        let r = self.expr(&f.body, &inner);
                        self.depth -= 1;
                        r?
                    }
                    None => self.builtin(name, &a)?,
                }
            }
        })
    }

    fn item(&mut self, it: &Item, scope: &Rc<Scope>, out: &mut Vec<Value>) -> Result<(), String> {
        match it {
            Item::E(e) => out.push(self.expr(e, scope)?),
            Item::Each(x) => {
                let mut inner = Vec::new();
                self.item(x, scope, &mut inner)?;
                for v in inner {
                    match v.items() {
                        Some(l) => out.extend(l),
                        None => out.push(v),
                    }
                }
            }
            Item::If(c, a, b) => {
                if self.expr(c, scope)?.truthy() {
                    self.item(a, scope, out)?;
                } else if let Some(b) = b {
                    self.item(b, scope, out)?;
                }
            }
            Item::Let(args, x) => {
                let inner = Scope::new(Some(scope.clone()));
                for a in args {
                    let v = self.expr(&a.value, &inner)?;
                    inner.set(a.name.as_deref().unwrap_or("_"), v);
                }
                self.item(x, &inner, out)?;
            }
            Item::For(args, x) => {
                let mut got = Vec::new();
                self.each(args, 0, scope, &mut |ev, s| ev.item(x, s, &mut got))?;
                out.extend(got);
            }
        }
        Ok(())
    }

    fn builtin(&mut self, name: &str, a: &Args) -> Result<Value, String> {
        let n = |k: usize| a.pos.get(k).and_then(Value::num);
        let f1 = |f: fn(f64) -> f64| n(0).map_or(Value::Undef, |x| Value::Num(f(x)));
        let deg = |x: f64| x.to_radians();
        Ok(match name {
            "sin" => f1(|x| clean_trig(x.to_radians().sin())),
            "cos" => f1(|x| clean_trig(x.to_radians().cos())),
            "tan" => n(0).map_or(Value::Undef, |x| Value::Num(deg(x).tan())),
            "asin" => f1(|x| x.asin().to_degrees()),
            "acos" => f1(|x| x.acos().to_degrees()),
            "atan" => f1(|x| x.atan().to_degrees()),
            "atan2" => match (n(0), n(1)) {
                (Some(y), Some(x)) => Value::Num(y.atan2(x).to_degrees()),
                _ => Value::Undef,
            },
            "abs" => f1(f64::abs),
            "sign" => f1(|x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }),
            "floor" => f1(f64::floor),
            "ceil" => f1(f64::ceil),
            "round" => f1(f64::round),
            "sqrt" => f1(f64::sqrt),
            "exp" => f1(f64::exp),
            "ln" => f1(f64::ln),
            "log" => match (n(0), n(1)) {
                (Some(b), Some(x)) => Value::Num(x.ln() / b.ln()),
                (Some(x), None) => Value::Num(x.log10()),
                _ => Value::Undef,
            },
            "pow" => match (n(0), n(1)) {
                (Some(x), Some(y)) => Value::Num(x.powf(y)),
                _ => Value::Undef,
            },
            "min" | "max" => {
                let vals: Vec<f64> = if a.pos.len() == 1 { a.pos[0].items().unwrap_or_default().iter().filter_map(Value::num).collect() } else { a.pos.iter().filter_map(Value::num).collect() };
                let r = if name == "min" { vals.iter().copied().fold(f64::INFINITY, f64::min) } else { vals.iter().copied().fold(f64::NEG_INFINITY, f64::max) };
                if vals.is_empty() { Value::Undef } else { Value::Num(r) }
            }
            "norm" => a.pos.first().and_then(Value::items).map_or(Value::Undef, |l| Value::Num(l.iter().filter_map(Value::num).map(|x| x * x).sum::<f64>().sqrt())),
            "cross" => match (a.pos.first().and_then(Value::vec3), a.pos.get(1).and_then(Value::vec3)) {
                (Some(x), Some(y)) => {
                    let c = x.cross(y);
                    Value::list(vec![Value::Num(c.x), Value::Num(c.y), Value::Num(c.z)])
                }
                _ => Value::Undef,
            },
            "len" => match a.pos.first() {
                Some(Value::Str(s)) => Value::Num(s.chars().count() as f64),
                Some(v) => v.items().map_or(Value::Undef, |l| Value::Num(l.len() as f64)),
                None => Value::Undef,
            },
            "concat" => {
                let mut out = Vec::new();
                for v in &a.pos {
                    match v {
                        Value::List(l) => out.extend(l.iter().cloned()),
                        other => out.push(other.clone()),
                    }
                }
                Value::list(out)
            }
            "str" => Value::Str(a.pos.iter().map(Value::show).collect()),
            "chr" => Value::Str(a.pos.iter().flat_map(|v| v.items().unwrap_or_else(|| vec![v.clone()])).filter_map(|v| v.num()).filter_map(|c| char::from_u32(c as u32)).collect()),
            "ord" => match a.pos.first() {
                Some(Value::Str(s)) if s.chars().count() == 1 => Value::Num(s.chars().next().map_or(0.0, |c| c as u32 as f64)),
                _ => Value::Undef,
            },
            "lookup" => {
                let (Some(key), Some(table)) = (n(0), a.pos.get(1).and_then(Value::items)) else { return Ok(Value::Undef) };
                let pts: Vec<(f64, f64)> = table.iter().filter_map(|r| r.vec2().map(|v| (v.x, v.y))).collect();
                if pts.is_empty() {
                    return Ok(Value::Undef);
                }
                if key <= pts[0].0 {
                    return Ok(Value::Num(pts[0].1));
                }
                for w in pts.windows(2) {
                    if key <= w[1].0 {
                        let t = (key - w[0].0) / (w[1].0 - w[0].0);
                        return Ok(Value::Num(w[0].1 + t * (w[1].1 - w[0].1)));
                    }
                }
                Value::Num(pts[pts.len() - 1].1)
            }
            "is_undef" => Value::Bool(matches!(a.pos.first(), None | Some(Value::Undef))),
            "is_num" => Value::Bool(matches!(a.pos.first(), Some(Value::Num(x)) if !x.is_nan())),
            "is_string" => Value::Bool(matches!(a.pos.first(), Some(Value::Str(_)))),
            "is_list" => Value::Bool(matches!(a.pos.first(), Some(Value::List(_)))),
            "is_bool" => Value::Bool(matches!(a.pos.first(), Some(Value::Bool(_)))),
            "is_function" => Value::Bool(false),
            "rands" => {
                let (lo, hi, count) = (n(0).unwrap_or(0.0), n(1).unwrap_or(1.0), n(2).unwrap_or(1.0) as usize);
                let mut s = n(3).unwrap_or(42.0).to_bits() | 1;
                Value::list((0..count.min(100_000)).map(|_| {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    Value::Num(lo + (hi - lo) * ((s >> 11) as f64 / (1u64 << 53) as f64))
                }).collect())
            }
            "version" => Value::list(vec![Value::Num(2021.0), Value::Num(1.0), Value::Num(0.0)]),
            "version_num" => Value::Num(20210100.0),
            "search" => {
                // The common case: where a value (or each of a string's letters) is in a list.
                let (Some(what), Some(list)) = (a.pos.first(), a.pos.get(1).and_then(Value::items)) else { return Ok(Value::Undef) };
                let find = |w: &Value| list.iter().position(|x| x == w || x.items().and_then(|l| l.first().cloned()).as_ref() == Some(w));
                match what {
                    Value::Num(_) => Value::list(find(what).map(|k| vec![Value::Num(k as f64)]).unwrap_or_default()),
                    other => Value::list(other.items().unwrap_or_default().iter().map(|w| find(w).map_or(Value::list(Vec::new()), |k| Value::Num(k as f64))).collect()),
                }
            }
            _ => {
                self.messages.push(format!("there's no function called {name}() — undef used"));
                Value::Undef
            }
        })
    }
}

/// sin / cos of whole-degree angles exactly (sin(180) is 0, not 1e-16).
fn clean_trig(x: f64) -> f64 {
    let r = x.round();
    if (x - r).abs() < 1e-12 { r } else { x }
}

fn neg(v: &Value) -> Value {
    match v {
        Value::Num(n) => Value::Num(-n),
        Value::List(l) => Value::list(l.iter().map(neg).collect()),
        _ => Value::Undef,
    }
}

fn binop(op: &str, a: &Value, b: &Value) -> Value {
    use Value::*;
    match op {
        "==" => return Bool(a == b),
        "!=" => return Bool(a != b),
        "<" | "<=" | ">" | ">=" => {
            let ord = match (a, b) {
                (Num(x), Num(y)) => x.partial_cmp(y),
                (Str(x), Str(y)) => Some(x.cmp(y)),
                (Bool(x), Bool(y)) => Some(x.cmp(y)),
                _ => None,
            };
            return match ord {
                Some(o) => Bool(match op {
                    "<" => o.is_lt(),
                    "<=" => o.is_le(),
                    ">" => o.is_gt(),
                    _ => o.is_ge(),
                }),
                None => Undef,
            };
        }
        _ => {}
    }
    match (a, b) {
        (Num(x), Num(y)) => Num(match op {
            "+" => x + y,
            "-" => x - y,
            "*" => x * y,
            "/" => x / y,
            "%" => x % y,
            "^" => x.powf(*y),
            _ => return Undef,
        }),
        (List(x), List(y)) if op == "+" || op == "-" => List(Rc::new(x.iter().zip(y.iter()).map(|(p, q)| binop(op, p, q)).collect())),
        (List(x), Num(_)) if op == "*" || op == "/" => List(Rc::new(x.iter().map(|p| binop(op, p, b)).collect())),
        (Num(_), List(y)) if op == "*" => List(Rc::new(y.iter().map(|q| binop(op, a, q)).collect())),
        (List(x), List(y)) if op == "*" => {
            let is_matrix = |l: &Vec<Value>| l.first().is_some_and(|r| matches!(r, List(_)));
            match (is_matrix(x), is_matrix(y)) {
                // Dot product.
                (false, false) => Num(x.iter().zip(y.iter()).filter_map(|(p, q)| Some(p.num()? * q.num()?)).sum()),
                // Matrix times vector.
                (true, false) => List(Rc::new(x.iter().map(|row| binop("*", row, b)).collect())),
                // Vector times matrix.
                (false, true) => {
                    let cols = y.first().and_then(Value::items).map_or(0, |r| r.len());
                    List(Rc::new((0..cols).map(|c| Num(x.iter().zip(y.iter()).filter_map(|(p, row)| Some(p.num()? * row.items()?.get(c)?.num()?)).sum())).collect()))
                }
                // Matrix times matrix.
                (true, true) => List(Rc::new(x.iter().map(|row| binop("*", row, b)).collect())),
            }
        }
        _ => Undef,
    }
}

/// The colour on the outside of a shape (moves don't hide it).
fn outer_color(g: &Geo) -> Option<[f32; 4]> {
    match g {
        Geo::Color(c, _) => Some(*c),
        Geo::Move(_, c) => outer_color(c),
        Geo::Union(c) => c.first().and_then(outer_color),
        _ => None,
    }
}

/// A CSS colour name or "#rrggbb" / "#rgb" as RGBA (0–1).
pub fn named_color(name: &str) -> Option<[f32; 4]> {
    let n = name.trim().to_ascii_lowercase();
    if let Some(hex) = n.strip_prefix('#') {
        let v = |s: &str| u8::from_str_radix(s, 16).ok().map(|x| x as f32 / 255.0);
        return match hex.len() {
            3 | 4 => {
                let d: Vec<String> = hex.chars().map(|c| format!("{c}{c}")).collect();
                Some([v(&d[0])?, v(&d[1])?, v(&d[2])?, d.get(3).and_then(|x| v(x)).unwrap_or(1.0)])
            }
            6 | 8 => Some([v(&hex[0..2])?, v(&hex[2..4])?, v(&hex[4..6])?, hex.get(6..8).and_then(v).unwrap_or(1.0)]),
            _ => None,
        };
    }
    let rgb: u32 = match n.as_str() {
        "black" => 0x000000, "white" => 0xffffff, "red" => 0xff0000, "lime" => 0x00ff00, "green" => 0x008000, "blue" => 0x0000ff,
        "yellow" => 0xffff00, "cyan" | "aqua" => 0x00ffff, "magenta" | "fuchsia" => 0xff00ff, "silver" => 0xc0c0c0, "gray" | "grey" => 0x808080,
        "maroon" => 0x800000, "olive" => 0x808000, "purple" => 0x800080, "teal" => 0x008080, "navy" => 0x000080, "orange" => 0xffa500,
        "gold" => 0xffd700, "pink" => 0xffc0cb, "brown" => 0xa52a2a, "tan" => 0xd2b48c, "beige" => 0xf5f5dc, "ivory" => 0xfffff0,
        "khaki" => 0xf0e68c, "coral" => 0xff7f50, "salmon" => 0xfa8072, "tomato" => 0xff6347, "orangered" => 0xff4500, "crimson" => 0xdc143c,
        "darkred" => 0x8b0000, "firebrick" => 0xb22222, "indianred" => 0xcd5c5c, "hotpink" => 0xff69b4, "deeppink" => 0xff1493,
        "violet" => 0xee82ee, "orchid" => 0xda70d6, "plum" => 0xdda0dd, "lavender" => 0xe6e6fa, "indigo" => 0x4b0082, "darkviolet" => 0x9400d3,
        "slateblue" => 0x6a5acd, "royalblue" => 0x4169e1, "dodgerblue" => 0x1e90ff, "deepskyblue" => 0x00bfff, "skyblue" => 0x87ceeb,
        "lightskyblue" => 0x87cefa, "lightblue" => 0xadd8e6, "steelblue" => 0x4682b4, "lightsteelblue" => 0xb0c4de, "cornflowerblue" => 0x6495ed,
        "cadetblue" => 0x5f9ea0, "powderblue" => 0xb0e0e6, "aliceblue" => 0xf0f8ff, "midnightblue" => 0x191970, "darkblue" => 0x00008b,
        "mediumblue" => 0x0000cd, "turquoise" => 0x40e0d0, "aquamarine" => 0x7fffd4, "darkcyan" => 0x008b8b, "lightcyan" => 0xe0ffff,
        "darkgreen" => 0x006400, "forestgreen" => 0x228b22, "seagreen" => 0x2e8b57, "limegreen" => 0x32cd32, "lightgreen" => 0x90ee90,
        "palegreen" => 0x98fb98, "springgreen" => 0x00ff7f, "olivedrab" => 0x6b8e23, "yellowgreen" => 0x9acd32, "chartreuse" => 0x7fff00,
        "darkolivegreen" => 0x556b2f, "mintcream" => 0xf5fffa, "honeydew" => 0xf0fff0, "chocolate" => 0xd2691e, "sienna" => 0xa0522d,
        "saddlebrown" => 0x8b4513, "peru" => 0xcd853f, "burlywood" => 0xdeb887, "wheat" => 0xf5deb3, "goldenrod" => 0xdaa520,
        "darkgoldenrod" => 0xb8860b, "darkorange" => 0xff8c00, "lightyellow" => 0xffffe0, "lemonchiffon" => 0xfffacd, "linen" => 0xfaf0e6,
        "snow" => 0xfffafa, "whitesmoke" => 0xf5f5f5, "gainsboro" => 0xdcdcdc, "lightgray" | "lightgrey" => 0xd3d3d3,
        "darkgray" | "darkgrey" => 0xa9a9a9, "dimgray" | "dimgrey" => 0x696969, "slategray" | "slategrey" => 0x708090,
        "lightslategray" | "lightslategrey" => 0x778899, "darkslategray" | "darkslategrey" => 0x2f4f4f, "ghostwhite" => 0xf8f8ff,
        "seashell" => 0xfff5ee, "mistyrose" => 0xffe4e1, "lightpink" => 0xffb6c1, "lightcoral" => 0xf08080, "rosybrown" => 0xbc8f8f,
        "mediumpurple" => 0x9370db, "darkmagenta" => 0x8b008b, "thistle" => 0xd8bfd8, "mediumseagreen" => 0x3cb371, "lightseagreen" => 0x20b2aa,
        "darkseagreen" => 0x8fbc8f, "mediumturquoise" => 0x48d1cc, "darkturquoise" => 0x00ced1, "paleturquoise" => 0xafeeee,
        "navajowhite" => 0xffdead, "moccasin" => 0xffe4b5, "bisque" => 0xffe4c4, "peachpuff" => 0xffdab9, "papayawhip" => 0xffefd5,
        "antiquewhite" => 0xfaebd7, "oldlace" => 0xfdf5e6, "floralwhite" => 0xfffaf0, "cornsilk" => 0xfff8dc, "azure" => 0xf0ffff,
        "lawngreen" => 0x7cfc00, "greenyellow" => 0xadff2f, "darkkhaki" => 0xbdb76b, "palegoldenrod" => 0xeee8aa, "sandybrown" => 0xf4a460,
        "darksalmon" => 0xe9967a, "lightsalmon" => 0xffa07a, "palevioletred" => 0xdb7093, "mediumvioletred" => 0xc71585,
        "darkorchid" => 0x9932cc, "mediumorchid" => 0xba55d3, "blueviolet" => 0x8a2be2, "darkslateblue" => 0x483d8b, "mediumslateblue" => 0x7b68ee,
        "lightgoldenrodyellow" => 0xfafad2, "mediumaquamarine" => 0x66cdaa, "mediumspringgreen" => 0x00fa9a, "lavenderblush" => 0xfff0f5,
        "transparent" => return Some([0.0, 0.0, 0.0, 0.0]),
        _ => return None,
    };
    let c = |s: u32| ((rgb >> s) & 0xff) as f32 / 255.0;
    Some([c(16), c(8), c(0), 1.0])
}

// ---------- hulls ----------

/// The convex hull of `pts` as triangles (indices into `pts`) wound
/// counter-clockwise seen from outside; None when the points are flat.
pub fn hull_3d(pts: &[DVec3]) -> Option<Vec<[usize; 3]>> {
    // Merge points that are the same.
    let scale = pts.iter().fold(0.0f64, |m, p| m.max(p.abs().max_element())).max(1.0);
    let eps = 1e-9 * scale;
    let mut uniq: Vec<usize> = Vec::new();
    for (i, p) in pts.iter().enumerate() {
        if !uniq.iter().any(|&j| pts[j].distance_squared(*p) < eps * eps) {
            uniq.push(i);
        }
    }
    if uniq.len() < 4 {
        return None;
    }
    // A first tetrahedron from points far apart.
    let p = |i: usize| pts[i];
    let a = *uniq.iter().min_by(|&&x, &&y| p(x).x.total_cmp(&p(y).x))?;
    let b = *uniq.iter().max_by(|&&x, &&y| p(x).distance_squared(p(a)).total_cmp(&p(y).distance_squared(p(a))))?;
    let line = (p(b) - p(a)).normalize_or_zero();
    let c = *uniq.iter().max_by(|&&x, &&y| {
        let dx = (p(x) - p(a)).reject_from_normalized(line).length_squared();
        let dy = (p(y) - p(a)).reject_from_normalized(line).length_squared();
        dx.total_cmp(&dy)
    })?;
    let n = (p(b) - p(a)).cross(p(c) - p(a));
    if n.length() < eps * scale {
        return None;
    }
    let d = *uniq.iter().max_by(|&&x, &&y| n.dot(p(x) - p(a)).abs().total_cmp(&n.dot(p(y) - p(a)).abs()))?;
    if n.dot(p(d) - p(a)).abs() / n.length() < eps * 10.0 {
        return None;
    }
    let inside = (p(a) + p(b) + p(c) + p(d)) / 4.0;
    let orient = |t: [usize; 3]| -> [usize; 3] {
        let nn = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
        if nn.dot(p(t[0]) - inside) < 0.0 { [t[0], t[2], t[1]] } else { t }
    };
    let mut faces: Vec<[usize; 3]> = vec![orient([a, b, c]), orient([a, b, d]), orient([a, c, d]), orient([b, c, d])];
    let above = |t: &[usize; 3], q: DVec3| {
        let nn = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
        let len = nn.length();
        len > 0.0 && nn.dot(q - p(t[0])) / len > eps * 10.0
    };
    for &i in &uniq {
        if [a, b, c, d].contains(&i) {
            continue;
        }
        let q = p(i);
        let seen: Vec<bool> = faces.iter().map(|t| above(t, q)).collect();
        if !seen.iter().any(|&s| s) {
            continue;
        }
        // The horizon: edges of seen faces whose other side isn't seen.
        let mut edges: HashMap<(usize, usize), usize> = HashMap::new();
        for (t, _) in faces.iter().zip(&seen).filter(|(_, s)| **s) {
            for k in 0..3 {
                *edges.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        let horizon: Vec<(usize, usize)> = edges.keys().filter(|(u, v)| !edges.contains_key(&(*v, *u))).copied().collect();
        faces = faces.into_iter().zip(seen).filter(|(_, s)| !s).map(|(t, _)| t).collect();
        for (u, v) in horizon {
            faces.push([u, v, i]);
        }
    }
    Some(faces)
}

/// The convex hull of 2-D points, counter-clockwise.
pub fn hull_2d(pts: &[DVec2]) -> Vec<DVec2> {
    let mut v: Vec<DVec2> = pts.to_vec();
    v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    v.dedup_by(|a, b| a.distance_squared(*b) < 1e-18);
    if v.len() < 3 {
        return v;
    }
    let cross = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut lower: Vec<DVec2> = Vec::new();
    for p in &v {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], *p) <= 1e-12 {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper: Vec<DVec2> = Vec::new();
    for p in v.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], *p) <= 1e-12 {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run1(src: &str) -> Output {
        run(src, None, &[]).unwrap()
    }

    fn eval(expr: &str) -> Value {
        let out = run(&format!("echo({expr});"), None, &[]).unwrap();
        let line = out.messages.last().unwrap().trim_start_matches("ECHO: ").to_string();
        parse_value(&line)
    }

    #[test]
    fn expressions() {
        assert_eq!(eval("1 + 2 * 3"), Value::Num(7.0));
        assert_eq!(eval("2 ^ 3 ^ 2"), Value::Num(512.0));
        assert_eq!(eval("-2 ^ 2"), Value::Num(-4.0));
        assert_eq!(eval("10 % 4"), Value::Num(2.0));
        assert_eq!(eval("true && !false ? 1 : 2"), Value::Num(1.0));
        assert_eq!(eval("[1,2,3] + [1,1,1]"), parse_value("[2, 3, 4]"));
        assert_eq!(eval("[1,2] * 2"), parse_value("[2, 4]"));
        assert_eq!(eval("[1,2,3] * [1,2,3]"), Value::Num(14.0));
        assert_eq!(eval("len([for (i=[0:4]) i])"), Value::Num(5.0));
        assert_eq!(eval("[for (i=[0:2:6]) if (i != 2) i*i]"), parse_value("[0, 16, 36]"));
        assert_eq!(eval("[for (x=[1,2], y=[10,20]) x+y]"), parse_value("[11, 21, 12, 22]"));
        assert_eq!(eval("[each [1,2], 3]"), parse_value("[1, 2, 3]"));
        assert_eq!(eval("let (a = 2, b = a * 3) a + b"), Value::Num(8.0));
        assert_eq!(eval("sin(90) + cos(180)"), Value::Num(0.0));
        assert_eq!(eval("max([3, 9, 2])"), Value::Num(9.0));
        assert_eq!(eval("min(4, 1, 7)"), Value::Num(1.0));
        assert_eq!(eval("str(\"a\", 1.5, [1,2])"), Value::Str("a1.5[1, 2]".into()));
        assert_eq!(eval("[3,4,5].y"), Value::Num(4.0));
        assert_eq!(eval("concat([1],[2,3],4)"), parse_value("[1, 2, 3, 4]"));
        assert_eq!(eval("norm([3,4])"), Value::Num(5.0));
        assert_eq!(eval("lookup(1.5, [[1,10],[2,20]])"), Value::Num(15.0));
        assert_eq!(fmt_num(1.0 / 3.0), "0.333333");
        assert_eq!(fmt_num(-12.5), "-12.5");
    }

    #[test]
    fn functions_and_recursion() {
        let out = run1("function fact(n) = n <= 1 ? 1 : n * fact(n - 1);\nfunction sum(v, i = 0) = i >= len(v) ? 0 : v[i] + sum(v, i + 1);\necho(fact(5), sum([1,2,3]));");
        assert_eq!(out.messages, vec!["ECHO: 120, 6".to_string()]);
    }

    #[test]
    fn modules_children_and_dollar_variables() {
        let src = "module two() { children(0); translate([5,0,0]) children(1); }\n\
                   module nut() cylinder(d=6, h=3);\n\
                   two() { cube(1); nut($fn=6); }";
        let out = run1(src);
        assert_eq!(out.parts.len(), 1);
        let Geo::Union(kids) = &out.parts[0].geo else { panic!("{:?}", out.parts[0].geo) };
        assert_eq!(kids[0], Geo::Cube { size: DVec3::ONE, center: false });
        let Geo::Move(Move::Translate(t), nut) = &kids[1] else { panic!() };
        assert_eq!(*t, DVec3::new(5.0, 0.0, 0.0));
        assert_eq!(**nut, Geo::Cylinder { h: 3.0, r1: 3.0, r2: 3.0, center: false, sides: Some(6) }, "$fn=6 passes into the module: a hexagon");
        // A big $fn is still truly round.
        let out = run1("$fn = 48; cylinder(r=2, h=1);");
        assert!(matches!(out.parts[0].geo, Geo::Cylinder { sides: None, .. }));
    }

    #[test]
    fn top_level_objects_are_parts_named_and_coloured() {
        let src = "part = \"all\";\nmodule half(side) cube(side == \"left\" ? 1 : 2);\n\
                   if (part == \"all\") {\n  color(\"skyblue\") half(\"left\");\n  color(\"white\") half(\"right\");\n  %cube(9);\n}";
        let out = run1(src);
        let names: Vec<&str> = out.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["half left", "half right"], "the % background object is left out");
        assert_eq!(out.parts[1].color, Some([1.0, 1.0, 1.0, 1.0]));
        // A parameter given another value picks the other branch.
        let out = run(src, None, &[("part".into(), Value::Str("none".into()))]).unwrap();
        assert!(out.parts.is_empty());
    }

    #[test]
    fn parameters_with_their_comments() {
        let src = "wall = 4;   // side wall thickness\nname = \"x\";\nsize = [1, 2];\nderived = wall * 2;\n$fn = 48;\nmodule m() { inner = 3; }";
        let p = params(src);
        let names: Vec<&str> = p.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["wall", "name", "size"], "computed values, $ variables and module locals aren't parameters");
        assert_eq!(p[0].note, "side wall thickness");
        assert_eq!(parse_value("12.5"), Value::Num(12.5));
        assert_eq!(parse_value("shell_left"), Value::Str("shell_left".into()));
        assert_eq!(parse_value("\"a\""), Value::Str("a".into()));
    }

    #[test]
    fn rotate_order_and_hull_points() {
        let out = run1("rotate([90, 0, 30]) cube(2);");
        let m = match &out.parts[0].geo {
            g => {
                fn mat(g: &Geo) -> DMat4 {
                    match g {
                        Geo::Move(mv, c) => mv.matrix() * mat(c),
                        _ => DMat4::IDENTITY,
                    }
                }
                mat(g)
            }
        };
        let want = DMat4::from_rotation_z(30f64.to_radians()) * DMat4::from_rotation_x(90f64.to_radians());
        assert!(m.abs_diff_eq(want, 1e-12), "X first, then Z");
        let pts = run1("hull() { cube(1); translate([3,0,0]) cube(1); }").parts[0].geo.hull_points(DMat4::IDENTITY).unwrap();
        assert_eq!(pts.len(), 16);
    }

    #[test]
    fn hulls() {
        // A cube's corners plus its middle: 12 triangles, the middle unused.
        let mut pts: Vec<DVec3> = (0..8).map(|k| DVec3::new((k & 1) as f64, ((k >> 1) & 1) as f64, ((k >> 2) & 1) as f64)).collect();
        pts.push(DVec3::splat(0.5));
        let tris = hull_3d(&pts).unwrap();
        assert!(!tris.iter().flatten().any(|&i| i == 8));
        // Closed and outward: the signed volume is the cube's.
        let vol: f64 = tris.iter().map(|t| pts[t[0]].dot(pts[t[1]].cross(pts[t[2]])) / 6.0).sum();
        assert!((vol - 1.0).abs() < 1e-9, "{vol}");
        assert!(hull_3d(&[DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::new(1.0, 1.0, 0.0)]).is_none(), "flat");
        let h = hull_2d(&[DVec2::ZERO, DVec2::X, DVec2::Y, DVec2::ONE, DVec2::splat(0.5)]);
        assert_eq!(h.len(), 4);
    }

    #[test]
    fn errors_say_where() {
        let e = run("cube(1);\ncube(2;", None, &[]).unwrap_err();
        assert!(e.contains("line 2"), "{e}");
        let e = run("module r() r();\nr();", None, &[]).unwrap_err();
        assert!(e.contains("too deeply"), "{e}");
        let out = run1("text(\"hi\"); frob();");
        assert_eq!(out.messages.len(), 2);
    }

    #[test]
    fn names_come_from_where_a_module_was_written() {
        // A loop variable where it's called doesn't change a global the module uses.
        let out = run1("x = 1;\nmodule m() cube(x);\nfor (x = [5]) m();");
        assert_eq!(out.parts[0].geo, Geo::Cube { size: DVec3::ONE, center: false });
        // But $ variables do pass down.
        let out = run1("module m() circle(r=1);\nm($fn=5);");
        assert!(matches!(out.parts[0].geo, Geo::Circle { sides: Some(5), .. }));
    }

    #[test]
    fn a_two_part_case_like_a_real_file() {
        // The shape of a real multi-part file: parameters, 2-D outlines
        // extruded, holes cut, halves split with intersection(), parts picked
        // by a `part` parameter, a background reference object.
        let src = r#"
part = "assembly";   // assembly | left | right
$fn = 48;
w = 100; h = 60; wall = 3; d = 20;
module outer2d() square([w + 2*wall, h + 2*wall], center=true);
module cavity2d() square([w, h], center=true);
module slot_y(cx, cz, sw, sh, y0, y1) hull() for (s=[-1,1]) translate([cx + s*(sw-sh)/2, y0, cz]) rotate([-90,0,0]) cylinder(d=sh, h=y1-y0);
module shell() difference() {
  linear_extrude(d) outer2d();
  translate([0,0,2]) linear_extrude(d) cavity2d();
  slot_y(10, 10, 12, 6, -h/2-wall-1, -h/2+1);
}
module half(side) intersection() {
  shell();
  if (side=="left") translate([-300,-300,-10]) cube([300,600,100]);
  else translate([0,-300,-10]) cube([300,600,100]);
}
pts = [ for (x=[-40,0,40]) [x, h/2-4], [-w/2+4, 0] ];
if (part=="left") half("left");
else if (part=="right") half("right");
else {
  color("lightskyblue") half("left");
  color("skyblue") half("right");
  %cube([w, h, 5], center=true);
}
"#;
        let out = run(src, None, &[]).unwrap();
        let names: Vec<&str> = out.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["half left", "half right"]);
        assert!(out.parts.iter().all(|p| p.color.is_some()));
        assert!(out.messages.is_empty(), "{:?}", out.messages);
        let out = run(src, None, &[("part".into(), Value::Str("right".into()))]).unwrap();
        assert_eq!(out.parts.len(), 1);
        assert_eq!(params(src).first().map(|p| (p.name.as_str(), p.note.as_str())), Some(("part", "assembly | left | right")));
        assert_eq!(eval_in(src, "len(pts)"), Value::Num(4.0));
    }

    fn eval_in(src: &str, expr: &str) -> Value {
        let out = run(&format!("{src}\necho({expr});"), None, &[]).unwrap();
        parse_value(out.messages.last().unwrap().trim_start_matches("ECHO: "))
    }
}
