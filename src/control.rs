//! Driving the app from outside, so changes can be checked without a person
//! at the mouse: `opencad --control PORT` listens on 127.0.0.1 for one JSON
//! request per line (screenshot, click, drag, key, type, state, …) and
//! answers each with one JSON line. Two clients use it:
//!
//! - `opencad ctl '<json>'` sends one request and prints the answer;
//! - `opencad mcp` is a Model Context Protocol server on stdin/stdout (for
//!   Claude Code), starting the app itself if it isn't running.
//!
//! Only this machine can connect (the listener is on the loopback address).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;

pub const DEFAULT_PORT: u16 = 47321;

/// A request from a client and where its answer goes.
pub type Request = (Value, mpsc::Sender<Value>);

/// Listen on `port`; requests arrive on the returned channel (the app takes
/// them each frame). `wake` asks the app to draw a frame so it sees them.
pub fn serve(port: u16, wake: impl Fn() + Send + Clone + 'static) -> std::io::Result<mpsc::Receiver<Request>> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let (tx, rx) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, wake) = (tx.clone(), wake.clone());
            std::thread::spawn(move || {
                let mut out = match stream.try_clone() {
                    Ok(s) => s,
                    Err(_) => return,
                };
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { return };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let answer = match serde_json::from_str::<Value>(&line) {
                        Ok(req) => {
                            let (reply_tx, reply_rx) = mpsc::channel();
                            if tx.send((req, reply_tx)).is_err() {
                                return; // the app has gone
                            }
                            wake();
                            reply_rx.recv().unwrap_or_else(|_| json!({"error": "the app closed"}))
                        }
                        Err(e) => json!({"error": format!("not JSON: {e}")}),
                    };
                    if writeln!(out, "{answer}").is_err() {
                        return;
                    }
                }
            });
        }
    });
    Ok(rx)
}

/// Send one request to the app on `port` and wait for its answer.
pub fn call(port: u16, req: &Value) -> Result<Value, String> {
    let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("can't reach Oxide360 on port {port} ({e}); start it with `oxide360 --control {port}`"))?;
    let mut out = stream.try_clone().map_err(|e| e.to_string())?;
    writeln!(out, "{req}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| format!("bad answer: {e}"))
}

/// An RGBA image as a PNG file's bytes.
pub fn png(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut enc = png::Encoder::new(&mut bytes, width as u32, height as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    if let Ok(mut w) = enc.write_header() {
        let _ = w.write_image_data(rgba);
    }
    bytes
}

pub fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                s.push(ABC[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// A key name as egui knows it ("Escape", "Enter", "A", "F6", "Delete", …).
pub fn key(name: &str) -> Option<egui::Key> {
    egui::Key::from_name(name).or_else(|| egui::Key::ALL.iter().copied().find(|k| k.name().eq_ignore_ascii_case(name)))
}

/// Modifier keys from a request's `"mods"` (e.g. ["shift", "ctrl"]).
pub fn modifiers(req: &Value) -> egui::Modifiers {
    let has = |m: &str| req["mods"].as_array().is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(m))));
    let ctrl = has("ctrl") || has("command");
    egui::Modifiers { alt: has("alt"), ctrl, shift: has("shift"), mac_cmd: false, command: ctrl }
}

pub fn button(req: &Value) -> egui::PointerButton {
    match req["button"].as_str().unwrap_or("left") {
        "right" => egui::PointerButton::Secondary,
        "middle" => egui::PointerButton::Middle,
        _ => egui::PointerButton::Primary,
    }
}

/// One frame's worth of made-up input, and the modifiers held during it.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub events: Vec<egui::Event>,
    pub mods: egui::Modifiers,
}

/// The input steps a request makes (one step per frame, so a press and its
/// release are seen as a click). `scale`: points per screenshot pixel, as
/// requests give positions in the screenshot's pixels.
pub fn steps(req: &Value, scale: f32) -> Result<Vec<Step>, String> {
    let mods = modifiers(req);
    let at = |v: &Value| -> Result<egui::Pos2, String> {
        match (v.get(0).and_then(Value::as_f64), v.get(1).and_then(Value::as_f64)) {
            (Some(x), Some(y)) => Ok(egui::pos2(x as f32 * scale, y as f32 * scale)),
            _ => Err("a position is [x, y] in screenshot pixels".into()),
        }
    };
    let xy = || at(&json!([req["x"], req["y"]]));
    let step = |events: Vec<egui::Event>| Step { events, mods };
    let press = |p: egui::Pos2, b: egui::PointerButton, pressed: bool| egui::Event::PointerButton { pos: p, button: b, pressed, modifiers: mods };
    let cmd = req["cmd"].as_str().unwrap_or("");
    Ok(match cmd {
        "move" => vec![step(vec![egui::Event::PointerMoved(xy()?)])],
        "click" | "double_click" => {
            let (p, b) = (xy()?, button(req));
            let mut v = vec![step(vec![egui::Event::PointerMoved(p)]), step(vec![press(p, b, true)]), step(vec![press(p, b, false)])];
            if cmd == "double_click" {
                v.extend([step(vec![press(p, b, true)]), step(vec![press(p, b, false)])]);
            }
            v
        }
        "drag" => {
            let (a, z, b) = (at(&req["from"])?, at(&req["to"])?, button(req));
            let n = req["steps"].as_u64().unwrap_or(12).clamp(2, 200) as usize;
            let mut v = vec![step(vec![egui::Event::PointerMoved(a)]), step(vec![press(a, b, true)])];
            v.extend((1..=n).map(|k| step(vec![egui::Event::PointerMoved(a.lerp(z, k as f32 / n as f32))])));
            v.push(step(vec![press(z, b, false)]));
            v
        }
        "scroll" => {
            let p = xy()?;
            let d = egui::vec2(req["dx"].as_f64().unwrap_or(0.0) as f32, req["dy"].as_f64().unwrap_or(0.0) as f32);
            vec![step(vec![egui::Event::PointerMoved(p)]), step(vec![egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: d, modifiers: mods, phase: egui::TouchPhase::Move }])]
        }
        "key" => {
            let name = req["key"].as_str().ok_or("key needs a \"key\" name, like \"Escape\" or \"L\"")?;
            let k = key(name).ok_or_else(|| format!("no key called {name:?}"))?;
            let ev = |pressed| egui::Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: mods };
            vec![step(vec![ev(true)]), step(vec![ev(false)])]
        }
        "type" => {
            let text = req["text"].as_str().ok_or("type needs \"text\"")?;
            vec![step(vec![egui::Event::Text(text.to_string())])]
        }
        other => return Err(format!("not an input command: {other:?}")),
    })
}

/// The tools the MCP server offers, with what they take.
fn mcp_tools() -> Value {
    let pos = json!({"type": "number"});
    let mods = json!({"type": "array", "items": {"type": "string", "enum": ["shift", "ctrl", "alt"]}, "description": "Modifier keys held"});
    json!([
        {"name": "screenshot", "description": "A picture of the Oxide360 window as it is now. Positions in other tools are in this picture's pixels.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "state", "description": "What's in the design and on screen, as JSON: timeline features and their errors, bodies and volumes, the open sketch, tool, selection, open dialogs, the last notice.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "click", "description": "Click at a point of the window (screenshot pixels).", "inputSchema": {"type": "object", "properties": {"x": pos, "y": pos, "button": {"type": "string", "enum": ["left", "right", "middle"]}, "double": {"type": "boolean"}, "mods": mods}, "required": ["x", "y"]}},
        {"name": "move", "description": "Move the pointer to a point (to hover).", "inputSchema": {"type": "object", "properties": {"x": pos, "y": pos}, "required": ["x", "y"]}},
        {"name": "drag", "description": "Press at `from`, move to `to`, release.", "inputSchema": {"type": "object", "properties": {"from": {"type": "array", "items": pos}, "to": {"type": "array", "items": pos}, "button": {"type": "string", "enum": ["left", "right", "middle"]}, "mods": mods}, "required": ["from", "to"]}},
        {"name": "scroll", "description": "Scroll the wheel at a point (dy > 0 scrolls up / zooms in).", "inputSchema": {"type": "object", "properties": {"x": pos, "y": pos, "dx": pos, "dy": pos, "mods": mods}, "required": ["x", "y", "dy"]}},
        {"name": "key", "description": "Press a key: egui names like \"Escape\", \"Enter\", \"Delete\", \"L\", \"F6\".", "inputSchema": {"type": "object", "properties": {"key": {"type": "string"}, "mods": mods}, "required": ["key"]}},
        {"name": "type", "description": "Type text into the focused box.", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}},
        {"name": "open", "description": "Open a design file (.ox360, or .ocad from before).", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
        {"name": "save", "description": "Save the design to a file.", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
        {"name": "import", "description": "Insert a STEP (.step/.stp) or STL file's bodies into the design.", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
        {"name": "export_drawing", "description": "Write the design's 2-D drawing sheet to a .pdf or .dxf file.", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
        {"name": "new", "description": "Throw the open design away and start an empty one (no questions).", "inputSchema": {"type": "object", "properties": {}}}
    ])
}

/// The control request an MCP tool call makes.
fn tool_request(name: &str, args: &Value) -> Value {
    let mut req = args.clone();
    if !req.is_object() {
        req = json!({});
    }
    let cmd = match name {
        "click" if args["double"].as_bool() == Some(true) => "double_click",
        other => other,
    };
    req["cmd"] = json!(cmd);
    req
}

/// Run the MCP server on stdin/stdout, talking to the app on `port`
/// (started with `--control port` if it isn't running).
pub fn mcp(port: u16) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let id = msg.get("id").cloned();
        let result: Result<Value, String> = match msg["method"].as_str().unwrap_or("") {
            "initialize" => Ok(json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "oxide360", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Drives the Oxide360 CAD app: take a screenshot first, then click/drag/key at positions in that screenshot; `state` lists the design."
            })),
            "tools/list" => Ok(json!({"tools": mcp_tools()})),
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("");
                let req = tool_request(name, &msg["params"]["arguments"]);
                Ok(match ensure_app(port).and_then(|_| call(port, &req)) {
                    Ok(answer) => tool_result(name, &answer),
                    Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
                })
            }
            "ping" => Ok(json!({})),
            _ if id.is_none() => continue, // a notification: nothing to answer
            m => Err(format!("unknown method {m}")),
        };
        let Some(id) = id else { continue };
        let reply = match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": e}}),
        };
        if writeln!(stdout, "{reply}").is_err() || stdout.flush().is_err() {
            break;
        }
    }
}

/// An MCP tool result from the app's answer: a screenshot as an image.
fn tool_result(name: &str, answer: &Value) -> Value {
    if let Some(e) = answer["error"].as_str() {
        return json!({"content": [{"type": "text", "text": e}], "isError": true});
    }
    if name == "screenshot" {
        if let Some(data) = answer["png_base64"].as_str() {
            let note = format!("{}×{} pixels", answer["width"], answer["height"]);
            return json!({"content": [{"type": "image", "data": data, "mimeType": "image/png"}, {"type": "text", "text": note}]});
        }
    }
    json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(answer).unwrap_or_default()}]})
}

/// Make sure the app is listening on `port`, starting it if not.
fn ensure_app(port: u16) -> Result<(), String> {
    if TcpStream::connect(("127.0.0.1", port)).is_ok() {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .args(["--control", &port.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("couldn't start Oxide360: {e}"))?;
    for _ in 0..150 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            // (Give its first frames a moment.)
            std::thread::sleep(std::time::Duration::from_millis(800));
            return Ok(());
        }
    }
    Err("started Oxide360, but it didn't start listening".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_click_is_move_press_release_in_separate_frames() {
        let s = steps(&json!({"cmd": "click", "x": 100, "y": 50, "mods": ["shift"]}), 0.5).unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].events, vec![egui::Event::PointerMoved(egui::pos2(50.0, 25.0))], "screenshot pixels to points");
        assert!(matches!(s[1].events[0], egui::Event::PointerButton { pressed: true, button: egui::PointerButton::Primary, .. }));
        assert!(matches!(s[2].events[0], egui::Event::PointerButton { pressed: false, .. }));
        assert!(s.iter().all(|st| st.mods.shift));
        assert_eq!(steps(&json!({"cmd": "double_click", "x": 1, "y": 1}), 1.0).unwrap().len(), 5);
        let d = steps(&json!({"cmd": "drag", "from": [0, 0], "to": [10, 0], "steps": 4, "button": "middle"}), 1.0).unwrap();
        assert_eq!(d.len(), 2 + 4 + 1);
        assert!(matches!(d[6].events[0], egui::Event::PointerButton { pressed: false, button: egui::PointerButton::Middle, pos, .. } if pos == egui::pos2(10.0, 0.0)));
        assert!(steps(&json!({"cmd": "key", "key": "Nope"}), 1.0).is_err());
        assert_eq!(steps(&json!({"cmd": "key", "key": "escape"}), 1.0).unwrap().len(), 2);
    }

    #[test]
    fn tool_calls_become_requests() {
        assert_eq!(tool_request("click", &json!({"x": 1, "y": 2, "double": true}))["cmd"], "double_click");
        assert_eq!(tool_request("state", &Value::Null), json!({"cmd": "state"}));
        let r = tool_result("screenshot", &json!({"png_base64": "AAAA", "width": 2, "height": 3}));
        assert_eq!(r["content"][0]["type"], "image");
        assert_eq!(tool_result("state", &json!({"error": "nope"}))["isError"], true);
    }

    #[test]
    fn requests_go_through_the_socket_and_back() {
        // A stand-in app answering each request with what it got.
        let rx = serve(47399, || {}).expect("port free");
        std::thread::spawn(move || {
            for (req, reply) in rx {
                let _ = reply.send(json!({"got": req}));
            }
        });
        let answer = call(47399, &json!({"cmd": "state"})).unwrap();
        assert_eq!(answer, json!({"got": {"cmd": "state"}}));
    }

    #[test]
    fn png_has_its_signature_and_size() {
        let p = png(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(p[16..20].try_into().unwrap()), 2);
    }
}
