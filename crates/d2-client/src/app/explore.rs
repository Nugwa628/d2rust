//! Bounded breadth-first UI exploration through autoplay-host's input paths.
//! Each edge replays its parent route on a fresh session; game state is never
//! restored by writing a snapshot. Findings are Unverified candidates.

use super::autoplay_front::FrontPhase;
use super::autoplay_host::{Command, Host, HostArgs, Session};
use super::single_player::{Character, GameData};
use crate::rules::camera::FrameSize;
use crate::ui::{Point, PointerButton};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

#[derive(Clone, Debug)]
enum Action {
    Click(String),
    Key(u32),
    Text(String),
    End(String, Point),
}
impl Action {
    fn label(&self) -> String {
        match self {
            Self::Click(id) => format!("click ui:{id}"),
            Self::Key(vk) => format!("key {vk}"),
            Self::Text(text) => format!("text {text}"),
            Self::End(id, p) => format!("slider ui:{id} {},{}", p.x, p.y),
        }
    }
}

trait Driver {
    fn reset(&mut self, route: &[Action]) -> Result<Value>;
    fn act(&mut self, action: &Action) -> Result<Value>;
    fn violations(&mut self) -> Vec<Value> {
        Vec::new()
    }
    /// The view after `route`, then the pointer moved onto element `id`
    /// (no click) and the same ticks as a click's: what the element shows
    /// on hover alone. `None`: the driver cannot tell.
    fn hover(&mut self, _route: &[Action], _id: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    /// The view after `route`, a click on element `id` (`click`) or only
    /// pointing at it, then the pointer moved off every element and the
    /// same ticks again: what stays once the hover is gone (a setting the
    /// click turned on that the hover only showed). `None`: the driver
    /// cannot tell.
    fn away(&mut self, _route: &[Action], _id: &str, _click: bool) -> Result<Option<Value>> {
        Ok(None)
    }
}

/// Whether a click on `id` only acts on content: an item slot or body
/// location does nothing while it and the cursor are empty (the items
/// sweep covers what it does with items).
fn needs_content(id: &str) -> bool {
    id.contains(".equip(") || (id.contains(".slot(") && !id.starts_with("quests."))
}

/// Why the screen says a click on `id` does nothing in this state on
/// purpose (`UiElement::idle`), or `None`.
fn idle<'a>(ui: &'a Value, id: &str) -> Option<&'a str> {
    ui["ids"].as_array()?.iter().find(|e| e["id"] == id)?["idle"].as_str()
}

/// Whether `after` (a click) differs from `hover` (only pointing at the
/// element, the same ticks later) in what the UI draws or what the client
/// sent: the click did something the ids and texts do not show (an image
/// value, an animation, a message to the server).
fn responded(after: &Value, hover: &Value) -> bool {
    after["digest"] != hover["digest"] || after["sent"] != hover["sent"]
}

/// The point a click on element `id` of `ui` uses.
fn click_point(ui: &Value, id: &str) -> Option<Point> {
    let e = ui["ids"].as_array()?.iter().find(|e| e["id"] == id)?;
    Some(Point::new(
        i32::try_from(e["at"][0].as_i64()?).ok()?,
        i32::try_from(e["at"][1].as_i64()?).ok()?,
    ))
}

/// A frame point outside every element of `ui`, scanning rows from the
/// top left in 8-pixel steps.
fn clear_point(ui: &Value) -> Point {
    let rects: Vec<[i64; 4]> = ui["ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(rect)
        .collect();
    (0..75)
        .flat_map(|y| (0..100).map(move |x| (x * 8, y * 8)))
        .find(|&(px, py)| {
            !rects
                .iter()
                .any(|&[x, y, w, h]| px >= x && py >= y && px < x + w && py < y + h)
        })
        .map_or(Point::new(0, 0), |(x, y)| Point::new(x as i32, y as i32))
}

fn key(ui: &Value) -> String {
    // ID order is a draw-order detail, not a distinct UI state.
    let mut ids: Vec<_> = ui["ids"]
        .as_array()
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    ids.sort_by_key(|e| e["id"].as_str().unwrap_or("").to_owned());
    json!([ui["screen"], ids, ui["observation"], ui["controls"]]).to_string()
}
fn screen(ui: &Value) -> String {
    let mut ids: Vec<_> = ui["ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["id"].as_str())
        .collect();
    ids.sort_unstable();
    json!([ui["screen"], ids]).to_string()
}
fn finding(rule: &str, state: &str, element: &str, detail: impl ToString) -> Value {
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(state.as_bytes()));
    json!({"rule":rule,"signature":format!("explore:{rule}:{}:{element}", &digest[..16]),
        "screen":state,"element":element,"detail":detail.to_string(),"status":"Unverified"})
}
fn rect(e: &Value) -> Option<[i64; 4]> {
    let a = e["rect"].as_array()?;
    Some([
        a.first()?.as_i64()?,
        a.get(1)?.as_i64()?,
        a.get(2)?.as_i64()?,
        a.get(3)?.as_i64()?,
    ])
}
fn layout(ui: &Value, size: FrameSize) -> Vec<Value> {
    let mut out = Vec::new();
    let state = screen(ui);
    let els = ui["ids"].as_array().cloned().unwrap_or_default();
    for (i, e) in els.iter().enumerate() {
        let id = e["id"].as_str().unwrap_or("?");
        let Some([x, y, w, h]) = rect(e) else {
            continue;
        };
        if x < 0
            || y < 0
            || w <= 0
            || h <= 0
            || x + w > i64::from(size.width)
            || y + h > i64::from(size.height)
        {
            out.push(finding("offscreen", &state, id, e["rect"].to_string()));
        }
        let at = &e["at"];
        if let (Some(px), Some(py)) = (at[0].as_i64(), at[1].as_i64()) {
            if px < x || py < y || px >= x + w || py >= y + h {
                out.push(finding(
                    "unreachable",
                    &state,
                    id,
                    format!("click point {at} outside {}", e["rect"]),
                ));
            }
        }
        for other in &els[i + 1..] {
            let Some([a, b, c, d]) = rect(other) else {
                continue;
            };
            if x < a + c && a < x + w && y < b + d && b < y + h {
                out.push(finding(
                    "overlap",
                    &state,
                    id,
                    format!("overlaps {}", other["id"]),
                ));
            }
        }
        if let Some([tx, ty, tw, th]) = e.get("text_rect").and_then(|r| rect(&json!({"rect":r}))) {
            if tx < x || ty < y || tx + tw > x + w || ty + th > y + h {
                out.push(finding(
                    "text_overflow",
                    &state,
                    id,
                    e["text_rect"].to_string(),
                ));
            }
        }
    }
    for (index, text) in ui["texts"].as_array().into_iter().flatten().enumerate() {
        if text["overflow"] == true {
            out.push(finding(
                "text_overflow",
                &state,
                &index.to_string(),
                format!("text {} exceeds box {}", text["rect"], text["box"]),
            ));
        }
        if text["missing_font"] == true {
            out.push(finding(
                "missing_font",
                &state,
                &index.to_string(),
                "text cannot be measured",
            ));
        }
    }
    out
}
fn actions(ui: &Value) -> Vec<Action> {
    let mut out = Vec::new();
    for e in ui["ids"].as_array().into_iter().flatten() {
        let Some(id) = e["id"].as_str() else { continue };
        if let Some(ends) = e["ends"].as_array() {
            for p in ends {
                if let (Some(x), Some(y)) = (p[0].as_i64(), p[1].as_i64()) {
                    out.push(Action::End(id.into(), Point::new(x as i32, y as i32)));
                }
            }
        } else {
            out.push(Action::Click(id.into()));
        }
    }
    if ui["screen"] == "char_create" && !ui["observation"].to_string().contains("Explorer") {
        out.push(Action::Text("Explorer".into()));
    }
    // Panel hotkeys use the player's normal binding path, seeding panels
    // which are accessible without a currently visible minipanel button.
    if ui["screen"] == "game" {
        out.extend([0x1B, 0x49, 0x43, 0x54, 0x51, 0x50, 0x4D, 0x4F, 0x0D, 0x09].map(Action::Key));
    }
    out
}

fn walk(
    driver: &mut impl Driver,
    size: FrameSize,
    max_states: usize,
    repeats: usize,
) -> Result<Value> {
    let root = driver.reset(&[])?;
    let root_key = key(&root);
    let mut queue = VecDeque::from([(Vec::<Action>::new(), root)]);
    let mut seen = BTreeSet::from([root_key]);
    let mut screens = BTreeMap::<String, usize>::new();
    let mut findings = Vec::new();
    let mut edges = Vec::new();
    let mut states = Vec::new();
    let mut limited = 0;
    while let Some((route, ui)) = queue.pop_front() {
        let state = screen(&ui);
        let visits = screens.entry(state.clone()).or_default();
        *visits += 1;
        if *visits > repeats {
            limited += 1;
            continue;
        }
        states.push(json!({"route":route.iter().map(Action::label).collect::<Vec<_>>(),"ui":ui}));
        findings.extend(layout(
            &ui,
            if ui["screen"] == "game" {
                size
            } else {
                FrameSize::D2RUST
            },
        ));
        if ui["screen"] == "exit" || ui["terminal"] == true {
            continue;
        }
        // Escape is checked independently of each close button.
        if !route.is_empty() {
            let escaped = driver
                .reset(&route)
                .and_then(|_| driver.act(&Action::Key(0x1B)));
            // A screen whose sub-state Escape backs out of (a key being
            // edited, a page without ids of its own) keeps its ids but not
            // its state.
            match escaped {
                Ok(after) if key(&after) == key(&ui) => findings.push(finding(
                    "escape",
                    &state,
                    "esc",
                    "Escape leaves the same screen",
                )),
                Err(e) => findings.push(finding("escape", &state, "esc", e)),
                _ => {}
            }
        }
        for action in actions(&ui) {
            let replay = match driver.reset(&route) {
                Ok(ui) => ui,
                Err(e) => {
                    findings.push(finding("replay", &state, &action.label(), e));
                    continue;
                }
            };
            if key(&replay) != key(&ui) {
                findings.push(finding(
                    "replay",
                    &state,
                    &action.label(),
                    "route did not restore the same UI",
                ));
                continue;
            }
            let before = key(&ui);
            match driver.act(&action) {
                Ok(after) => {
                    let changed = key(&after) != before;
                    let close = action.label().contains(".close")
                        || action.label().contains(".exit")
                        || action.label().contains(".cancel");
                    if !changed && close {
                        findings.push(finding(
                            "unreachable_close",
                            &state,
                            &action.label(),
                            "close input left the same screen",
                        ));
                    }
                    let quiet = match &action {
                        Action::Click(id)
                            if !changed
                                && !close
                                && !needs_content(id)
                                && idle(&ui, id).is_none() =>
                        {
                            match driver.hover(&route, id) {
                                Ok(Some(hover)) if responded(&after, &hover) => false,
                                // The hover may show what the click turned
                                // on; compare both once the pointer left.
                                Ok(Some(_)) => match (
                                    driver.away(&route, id, true),
                                    driver.away(&route, id, false),
                                ) {
                                    (Ok(Some(click)), Ok(Some(hover))) => {
                                        !responded(&click, &hover)
                                    }
                                    (Err(e), _) | (_, Err(e)) => {
                                        findings.push(finding(
                                            "replay",
                                            &state,
                                            &action.label(),
                                            e,
                                        ));
                                        false
                                    }
                                    _ => true,
                                },
                                Ok(None) => true,
                                Err(e) => {
                                    findings.push(finding("replay", &state, &action.label(), e));
                                    false
                                }
                            }
                        }
                        _ => false,
                    };
                    if quiet {
                        findings.push(finding(
                            "no_response",
                            &state,
                            &action.label(),
                            "listed element produced no observable UI change",
                        ));
                    }
                    edges.push(json!({"route":route.iter().map(Action::label).collect::<Vec<_>>(),"action":action.label(),"changed":changed,"after":after}));
                    findings.extend(driver.violations());
                    if seen.insert(key(&after)) {
                        if seen.len() <= max_states {
                            let mut next = route.clone();
                            next.push(action);
                            queue.push_back((next, after));
                        } else {
                            limited += 1;
                        }
                    }
                }
                Err(e) => findings.push(finding("input", &state, &action.label(), e)),
            }
        }
    }
    let mut signatures = BTreeSet::new();
    findings.retain(|v| signatures.insert(v["signature"].to_string()));
    Ok(
        json!({"format":"explore 1","tick_rate":60,"resolution":[size.width,size.height],
        "states":seen.len(),"screens":screens.len(),"limited":limited,"complete":limited==0,
        "findings":findings,"edges":edges,"visited":states}),
    )
}

/// Font-table measurements use the same glyph layout as the renderer.
struct TextMetrics {
    fonts: BTreeMap<u16, d2_formats::font::FontTable>,
    strings: super::strings::TableStrings,
}
impl TextMetrics {
    fn load(data: &GameData) -> Result<Self> {
        let GameData::Live(live) = data;
        let mut fonts = BTreeMap::new();
        for id in 0..14 {
            if let Some(info) = crate::ui::text::font_info(id) {
                if let Some(Ok(font)) =
                    crate::assets::path::read_font_table(live.archives.as_ref(), info.tbl_path)
                {
                    fonts.insert(id, font);
                }
            }
        }
        Ok(Self {
            fonts,
            strings: super::strings::TableStrings::load(live.archives.as_ref(), "eng")?,
        })
    }
    fn width(&self, id: u16, text: &[u16]) -> i32 {
        self.fonts.get(&id).map_or(0, |font| {
            crate::ui::front_end::glyphs::text_width(font, text)
        })
    }
    fn row(
        &self,
        font: u16,
        box_rect: [i32; 4],
        glyphs: Vec<(Point, usize)>,
        content: String,
    ) -> Value {
        let Some(table) = self.fonts.get(&font) else {
            return json!({"missing_font":true,"content":content});
        };
        let mut bounds: Option<[i32; 4]> = None;
        for (at, index) in glyphs {
            let Some(g) = table.glyphs.get(index) else {
                continue;
            };
            let b = [
                at.x,
                at.y - i32::from(g.height) + 1,
                at.x + i32::from(g.width),
                at.y + 1,
            ];
            bounds = Some(match bounds {
                None => b,
                Some(old) => [
                    old[0].min(b[0]),
                    old[1].min(b[1]),
                    old[2].max(b[2]),
                    old[3].max(b[3]),
                ],
            });
        }
        let rect = bounds.unwrap_or([box_rect[0], box_rect[1], box_rect[0], box_rect[1]]);
        let overflow = rect[0] < box_rect[0]
            || rect[1] < box_rect[1]
            || rect[2] > box_rect[0] + box_rect[2]
            || rect[3] > box_rect[1] + box_rect[3];
        json!({"rect":rect,"box":box_rect,"overflow":overflow,"content":content})
    }
    fn front(&self, draws: &[crate::ui::front_end::DrawItem]) -> Vec<Value> {
        use crate::ui::front_end::DrawItem;
        draws
            .iter()
            .filter_map(|draw| {
                let DrawItem::Text {
                    font,
                    at,
                    label,
                    boxed,
                    string_id,
                    text,
                    ..
                } = draw
                else {
                    return None;
                };
                let box_rect = if let Some(label) = label {
                    [
                        at.x,
                        at.y - i32::from(label.h),
                        i32::from(label.w),
                        i32::from(label.h),
                    ]
                } else if let Some(b) = boxed {
                    [at.x, at.y - i32::from(b.h), i32::from(b.w), i32::from(b.h)]
                } else {
                    [0, 0, 800, 600]
                };
                let quads = crate::ui::front_end::glyphs::text_quads(
                    draw,
                    &|id| self.strings.by_id(id as u16),
                    &|id| self.fonts.get(&id).cloned(),
                );
                let table = self.fonts.get(font);
                let glyphs = quads
                    .into_iter()
                    .filter_map(|q| {
                        table?
                            .glyphs
                            .iter()
                            .position(|g| g.frame == q.frame)
                            .map(|index| (q.at, index))
                    })
                    .collect();
                // Use text and string ID as a stable observation, never animated art.
                Some(self.row(*font, box_rect, glyphs, format!("{string_id}:{text}")))
            })
            .collect()
    }
    fn game(&self, draws: &[crate::ui::draw::UiDraw]) -> Vec<Value> {
        use crate::ui::draw::UiDraw;
        draws
            .iter()
            .filter_map(|draw| {
                let UiDraw::Text(t) = draw else { return None };
                let glyphs = self
                    .fonts
                    .get(&t.style.font)
                    .and_then(|font| {
                        crate::ui::text::layout_text(
                            font,
                            &t.text,
                            t.at,
                            t.style,
                            &t.opts,
                            &crate::ui::text::OriginalText,
                        )
                        .ok()
                    })
                    .unwrap_or_default();
                Some(self.row(
                    t.style.font,
                    [t.clip.x, t.clip.y, i32::from(t.clip.w), i32::from(t.clip.h)],
                    glyphs.into_iter().map(|g| (g.at, g.record)).collect(),
                    String::from_utf16_lossy(&t.text),
                ))
            })
            .collect()
    }
}

struct Live {
    args: HostArgs,
    data: GameData,
    character: Option<Character>,
    session: Option<Session>,
    checks: super::repro::invariants::Checks,
    scenario: crate::scenario_script::Scenario,
    log: super::repro::StepLog,
    coverage: super::coverage::Coverage,
    metrics: TextMetrics,
    captured: Vec<Value>,
    front_work: PathBuf,
    resets: u64,
}
impl Live {
    fn view(&mut self) -> Result<Value> {
        let mut ui = self.command(Command::Ui)?;
        let digest = |draws: String| {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(draws.as_bytes()))
        };
        let texts = match self.session.as_mut().context("session")? {
            Session::Front(front) => {
                ui["controls"] = front.explorer_controls();
                let draws = front.explorer_draws(&|id, text| self.metrics.width(id, text));
                ui["digest"] = json!(digest(format!("{draws:?}")));
                self.metrics.front(&draws)
            }
            Session::Game(host) => {
                ui["sent"] = json!(host.sent_counts());
                let world = host.app_mut().world();
                let bridge = world.resource::<crate::bridge::BridgeResource>();
                let panels = world.non_send::<crate::world_view::WorldViewUi>();
                let ctx = crate::ui::UiCtx {
                    tick: bridge.0.world().frames,
                    world: bridge.0.world(),
                    strings: &*panels.strings,
                };
                let mut draws = Vec::new();
                panels.root.draw(&ctx, &mut draws);
                // Measure where the frame shows them, as the element ids are.
                crate::world_view::ui_bind::place_in_frame(&mut draws, FrameSize::play());
                ui["digest"] = json!(digest(format!("{draws:?}")));
                self.metrics.game(&draws)
            }
        };
        ui["terminal"] = json!(self.args.front && matches!(self.session, Some(Session::Game(_))));
        ui["texts"] = json!(texts);
        ui["observation"] = json!(texts.iter().map(|t| &t["content"]).collect::<Vec<_>>());
        Ok(ui)
    }
    fn command(&mut self, command: Command) -> Result<Value> {
        self.session
            .as_mut()
            .context("session")?
            .run(command)?
            .context("session ended")
    }
    fn check(&mut self) -> Result<()> {
        if let Some(Session::Game(host)) = &mut self.session {
            let tick = host.passes() as u32;
            // The explorer runs no scenario pokes: no goto is walking.
            self.checks
                .tick(host, &self.scenario, tick, &self.log, &[])?;
        }
        Ok(())
    }
    fn collect(&mut self) -> Result<()> {
        if let Some(Session::Game(host)) = &mut self.session {
            self.coverage.merge(&super::coverage::snapshot(host)?);
        }
        self.captured.append(&mut self.checks.violations);
        Ok(())
    }
}
impl Driver for Live {
    fn reset(&mut self, route: &[Action]) -> Result<Value> {
        self.collect()?;
        self.session = None;
        self.session = Some(if self.args.front {
            // Normal menu actions may create/delete saves. Replay against an
            // isolated copy so both the seed folder and later routes stay fixed.
            let folder = self.front_work.join(self.resets.to_string());
            self.resets += 1;
            std::fs::create_dir_all(&self.front_work)?;
            std::fs::create_dir(&folder)?;
            for entry in std::fs::read_dir(self.args.save_dir.as_ref().context("save folder")?)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_file()
                    && path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("d2s"))
                {
                    std::fs::copy(&path, folder.join(entry.file_name()))?;
                }
            }
            let mut args = self.args.clone();
            args.save_dir = Some(folder);
            Session::Front(Box::new(FrontPhase::start_with(&args, self.data.clone())?))
        } else {
            Session::Game(Box::new(Host::start_with_coverage(
                self.data.clone(),
                self.character.clone().context("character")?,
                None,
                false,
                self.args.seed,
            )?))
        });
        self.checks = Default::default();
        self.log = Default::default();
        self.command(Command::Step(8))?;
        self.check()?;
        for action in route {
            let ui = self.act(action)?;
            if ui["screen"] == "exit" {
                return Ok(ui);
            }
        }
        self.view()
    }
    fn act(&mut self, action: &Action) -> Result<Value> {
        match action {
            Action::Click(id) => {
                self.command(Command::ClickUi(id.clone()))?;
            }
            Action::Text(text) => {
                self.command(Command::Text(text.clone()))?;
            }
            Action::Key(vk) => {
                self.command(Command::Key(*vk))?;
            }
            Action::End(_, p) => {
                self.command(Command::Press(PointerButton::Left, *p))?;
                self.command(Command::Step(1))?;
                self.command(Command::Release(PointerButton::Left, *p))?;
            }
        }
        self.check()?;
        if let Err(e) = self.command(Command::Step(2)) {
            if e.to_string().contains("front end exited (Exit Game)") {
                return Ok(json!({"k":"ui","screen":"exit","ids":[]}));
            }
            return Err(e);
        }
        self.check()?;
        // Save and Exit Game in a game: the client asked to leave (C→S
        // 0x69); the game app ends on the server's answer, which this host
        // does not wait for.
        if let Some(Session::Game(host)) = &mut self.session {
            let world = host.app_mut().world();
            if world
                .resource::<crate::bridge::BridgeResource>()
                .0
                .world()
                .exit_requested
            {
                return Ok(json!({"k":"ui","screen":"exit","ids":[]}));
            }
        }
        self.view()
    }
    fn hover(&mut self, route: &[Action], id: &str) -> Result<Option<Value>> {
        let ui = self.reset(route)?;
        let Some(at) = click_point(&ui, id) else {
            return Ok(None);
        };
        self.command(Command::Move(at))?;
        self.check()?;
        self.command(Command::Step(2))?;
        self.check()?;
        self.view().map(Some)
    }
    fn away(&mut self, route: &[Action], id: &str, click: bool) -> Result<Option<Value>> {
        let ui = self.reset(route)?;
        let Some(at) = click_point(&ui, id) else {
            return Ok(None);
        };
        if click {
            self.act(&Action::Click(id.into()))?;
        } else {
            self.command(Command::Move(at))?;
            self.check()?;
            self.command(Command::Step(2))?;
            self.check()?;
        }
        self.command(Command::Move(clear_point(&ui)))?;
        self.check()?;
        self.command(Command::Step(2))?;
        self.check()?;
        self.view().map(Some)
    }
    fn violations(&mut self) -> Vec<Value> {
        let mut all = std::mem::take(&mut self.captured);
        all.append(&mut self.checks.violations);
        all
    }
}

/// Run one resolution in this process. Use separate invocations for each frame
/// size, since the game's frame size is fixed for the life of a process.
pub fn main(args: &[String]) -> Result<()> {
    let mut host_args = Vec::new();
    let mut offscreen = false;
    let mut out = PathBuf::from("explore-out");
    let (mut max_states, mut repeats) = (128usize, 2usize);
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--game" => {}
            "--offscreen" => offscreen = true,
            "--out" => out = it.next().context("--out DIR")?.into(),
            "--max-states" => max_states = it.next().context("--max-states N")?.parse()?,
            "--repeats" => repeats = it.next().context("--repeats N")?.parse()?,
            _ => host_args.push(flag.clone()),
        }
    }
    if max_states == 0 || repeats == 0 {
        bail!("limits must be positive")
    }
    if !host_args
        .iter()
        .any(|v| matches!(v.as_str(), "--front" | "--new" | "--save"))
    {
        host_args.extend(["--new".into(), "amazon".into(), "Explorer".into()]);
    }
    if host_args.iter().any(|v| v == "--front") && !host_args.iter().any(|v| v == "--save-dir") {
        let saves = out.join("saves");
        std::fs::create_dir_all(&saves)?;
        host_args.extend(["--save-dir".into(), saves.to_string_lossy().into_owned()]);
    }
    let mut options = super::autoplay_host::parse_args(&host_args)?;
    options.seed.get_or_insert(7);
    if offscreen && options.front {
        bail!("--offscreen captures in-game routes; run --front without it")
    }
    let size = options.res.unwrap_or(FrameSize::D2RUST);
    FrameSize::set_play(size)?;
    let dir = options
        .game_dir
        .clone()
        .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from));
    let data = GameData::select(dir.as_deref())?;
    let character = if options.front {
        None
    } else {
        Some(
            super::play_start::resolve(
                &super::play_start::CliStart {
                    save: options.save.clone(),
                    new: options.new.clone(),
                    save_dir: None,
                    difficulty: options.difficulty,
                    hardcore: false,
                },
                &data,
                dir.as_deref(),
                None,
                None,
            )?
            .character,
        )
    };
    let metrics = TextMetrics::load(&data)?;
    let mut live = Live {
        args: options.clone(),
        data,
        character,
        session: None,
        checks: Default::default(),
        scenario: crate::scenario_script::Scenario::parse(
            "scenario 2\ntick_rate 60\nname explore\nseed 7\nchar class 0\nend 1\n",
        )
        .map_err(|e| anyhow!(e))?,
        log: Default::default(),
        coverage: Default::default(),
        metrics,
        captured: Vec::new(),
        front_work: out.join(format!("sessions-{}", std::process::id())),
        resets: 0,
    };
    let wall = std::time::Instant::now();
    let capture = super::checks::capture::Capture::default();
    let result = bevy::log::tracing::dispatcher::with_default(&capture.dispatch(), || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            walk(&mut live, size, max_states, repeats)
        }))
    });
    let mut report = match result {
        Ok(Ok(report)) => report,
        Ok(Err(e)) => {
            json!({"format":"explore 1","complete":false,"findings":[finding("run","session","",e)]})
        }
        Err(payload) => {
            json!({"format":"explore 1","complete":false,"findings":[finding("panic","session","",super::checks::capture::panic_text(payload.as_ref()))]})
        }
    };
    if let Err(e) = live.collect() {
        report["findings"]
            .as_array_mut()
            .context("findings")?
            .push(finding("coverage", "session", "collect", e));
    }
    for v in live.violations().into_iter().chain(
        capture
            .take()
            .into_iter()
            .map(|v| json!({"rule":v.kind,"signature":v.signature(),"detail":v.detail})),
    ) {
        report["findings"]
            .as_array_mut()
            .context("findings")?
            .push(v);
    }
    report["coverage"] = serde_json::to_value(&live.coverage)?;
    if offscreen {
        std::fs::create_dir_all(&out)?;
        match capture_routes(&options, &out, &report) {
            Ok(count) => report["offscreen_shots"] = json!(count),
            Err(e) => report["findings"]
                .as_array_mut()
                .context("findings")?
                .push(finding("offscreen", "capture", "", e)),
        }
    }
    report["seed"] = json!(options.seed);
    report["mode"] = json!(if options.front { "front" } else { "game" });
    report["limits"] = json!({"max_states": max_states, "repeats": repeats});
    report["wall_ms"] = json!(wall.elapsed().as_millis() as u64);
    report["text_metrics_available"] = json!(true);
    std::fs::create_dir_all(&out)?;
    std::fs::write(
        out.join("explore.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "explore: {} states, {} findings, {} ms",
        report["states"],
        report["findings"].as_array().map_or(0, Vec::len),
        report["wall_ms"]
    );
    if report["findings"].as_array().is_some_and(|f| !f.is_empty()) || report["complete"] != true {
        bail!(
            "Unverified findings or exploration limit; see {}",
            out.join("explore.json").display()
        );
    }
    Ok(())
}

/// Replay discovered states in T5's hidden 16:9 renderer, through T8's
/// stdio control channel. No OS pointer or focus API is used.
fn capture_routes(options: &HostArgs, out: &std::path::Path, report: &Value) -> Result<usize> {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command as Process, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut count = 0;
    let shots = std::fs::canonicalize(out)?.join("shots");
    std::fs::create_dir_all(&shots)?;
    for (index, state) in report["visited"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let folder = shots.join(index.to_string());
        std::fs::create_dir_all(&folder)?;
        let mut process = Process::new(std::env::current_exe()?);
        process
            .args([
                "play",
                "--offscreen",
                "--size",
                "1920x1080",
                "--control",
                "stdio",
                "--no-sound",
                "--res",
            ])
            .arg(format!(
                "{}x{}",
                options.res.unwrap_or(FrameSize::D2RUST).width,
                options.res.unwrap_or(FrameSize::D2RUST).height
            ))
            .arg("--save-dir")
            .arg(&folder)
            .env("D2RUST_HOME", folder.join("user"))
            .env("D2RUST_NO_DIALOG", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(folder.join("stderr.log"))?);
        if let Some(dir) = &options.game_dir {
            process.arg("--game-dir").arg(dir);
        }
        if let Some(save) = &options.save {
            process.arg("--save").arg(save);
        } else if let Some((class, name)) = &options.new {
            process.args(["--new", class, name]);
        }
        if let Some(seed) = options.seed {
            process.arg("--seed").arg(seed.to_string());
        }
        process
            .arg("--difficulty")
            .arg(["normal", "nightmare", "hell"][usize::from(options.difficulty)]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            process.creation_flags(0x08000000);
        }
        let mut child = Child(process.spawn()?);
        let mut input = child.0.stdin.take().context("control stdin")?;
        let output = child.0.stdout.take().context("control stdout")?;
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if line.starts_with('{') && tx.send(line).is_err() {
                    break;
                }
            }
        });
        let reply = || -> Result<Value> {
            let line = rx
                .recv_timeout(Duration::from_secs(120))
                .context("offscreen control reply")?;
            let value: Value = serde_json::from_str(&line)?;
            if value["k"] == "error" {
                bail!("offscreen: {value}")
            }
            Ok(value)
        };
        let hello = reply()?;
        if hello["k"] != "hello" {
            bail!("offscreen hello: {hello}")
        }
        let mut send = |line: &str| -> Result<Value> {
            writeln!(input, "{line}")?;
            input.flush()?;
            reply()
        };
        send("step 8")?;
        for action in state["route"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(rest) = action.strip_prefix("slider ") {
                let (_, point) = rest.rsplit_once(' ').context("slider point")?;
                let (x, y) = point.split_once(',').context("slider coordinates")?;
                send(&format!("press L {x} {y}"))?;
                send("step 1")?;
                send(&format!("release L {x} {y}"))?;
            } else {
                send(action)?;
            }
            send("step 2")?;
        }
        let actual = send("ui")?;
        if screen(&actual) != screen(&state["ui"]) {
            bail!("offscreen route {index} reached a different UI: {actual}")
        }
        send(&format!("shot {}", folder.join("frame.png").display()))?;
        send("quit")?;
        child.0.wait()?;
        reader
            .join()
            .map_err(|_| anyhow!("offscreen output reader panicked"))?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Planted {
        page: bool,
    }
    impl Planted {
        fn ui(&self) -> Value {
            json!({"screen":if self.page {"panel"} else {"menu"},"ids":[{
                "id":if self.page {"panel.close"} else {"menu.open"},"rect":[0,0,10,10],"at":[5,5]
            }]})
        }
    }
    impl Driver for Planted {
        fn reset(&mut self, route: &[Action]) -> Result<Value> {
            self.page = false;
            for a in route {
                self.act(a)?;
            }
            Ok(self.ui())
        }
        fn act(&mut self, action: &Action) -> Result<Value> {
            if matches!(action,Action::Click(id) if id=="menu.open") {
                self.page = true;
            }
            if matches!(action, Action::Key(0x1B)) {
                self.page = false;
            }
            Ok(self.ui())
        }
    }
    #[test]
    fn bfs_reports_planted_unreachable_close_even_when_escape_works() {
        let report = walk(&mut Planted { page: false }, FrameSize::D2RUST, 10, 2).unwrap();
        assert_eq!(report["states"], 2);
        assert!(report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["rule"] == "unreachable_close"));
    }
    #[test]
    fn terminal_handoff_is_a_leaf() {
        struct Terminal;
        impl Driver for Terminal {
            fn reset(&mut self, _: &[Action]) -> Result<Value> {
                Ok(json!({"screen":"game", "terminal":true, "ids":[{"id":"panel.close"}]}))
            }
            fn act(&mut self, _: &Action) -> Result<Value> {
                panic!("terminal state expanded")
            }
        }
        let report = walk(&mut Terminal, FrameSize::D2RUST, 10, 2).unwrap();
        assert!(report["edges"].as_array().unwrap().is_empty());
    }
    #[test]
    fn bounded_walk_does_not_claim_completion() {
        let report = walk(&mut Planted { page: false }, FrameSize::D2RUST, 1, 2).unwrap();
        assert_eq!(report["complete"], false);
        assert_eq!(report["limited"], 1);
        assert_eq!(report["visited"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn screen_limit_ignores_changing_control_values() {
        let a = json!({"screen":"options","ids":[{"id":"volume"}],"observation":["10"]});
        let b = json!({"screen":"options","ids":[{"id":"volume"}],"observation":["90"]});
        assert_ne!(key(&a), key(&b));
        assert_eq!(screen(&a), screen(&b));
    }
    /// Escape that closes a page without ids of its own (the help screen)
    /// changes the state though not the ids: no finding. Escape that
    /// changes nothing is one.
    #[test]
    fn escape_is_judged_on_the_whole_state() {
        struct Help {
            open: bool,
            esc_closes: bool,
        }
        impl Driver for Help {
            fn reset(&mut self, route: &[Action]) -> Result<Value> {
                self.open = false;
                for a in route {
                    self.act(a)?;
                }
                Ok(self.view())
            }
            fn act(&mut self, action: &Action) -> Result<Value> {
                match action {
                    Action::Click(id) if id == "hud.help" => self.open = true,
                    Action::Key(0x1B) if self.esc_closes => self.open = false,
                    _ => {}
                }
                Ok(self.view())
            }
        }
        impl Help {
            fn view(&self) -> Value {
                let text = if self.open { "Help" } else { "" };
                json!({"screen":"game","ids":[{"id":"hud.help","rect":[0,0,10,10],"at":[5,5]}],
                    "observation":[text]})
            }
        }
        for esc_closes in [true, false] {
            let mut d = Help {
                open: false,
                esc_closes,
            };
            let report = walk(&mut d, FrameSize::D2RUST, 10, 2).unwrap();
            let escapes = report["findings"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["rule"] == "escape")
                .count();
            assert_eq!(escapes, usize::from(!esc_closes));
        }
    }
    /// A click whose only effect is an image (a value drawn as art), a
    /// message to the server, or nothing, next to an item slot: only the
    /// click that does nothing is a no_response, and the empty slot is
    /// not one (its click needs an item).
    #[test]
    fn no_response_compares_the_click_with_a_hover() {
        #[derive(Default)]
        struct Art {
            value: u8,
            sent: u8,
        }
        impl Driver for Art {
            fn reset(&mut self, _: &[Action]) -> Result<Value> {
                *self = Art::default();
                Ok(self.view())
            }
            fn act(&mut self, action: &Action) -> Result<Value> {
                match action {
                    Action::Click(id) if id == "options.light" => self.value += 1,
                    Action::Click(id) if id == "belt.potion" => self.sent += 1,
                    _ => {}
                }
                Ok(self.view())
            }
            fn hover(&mut self, route: &[Action], _: &str) -> Result<Option<Value>> {
                self.reset(route).map(Some)
            }
        }
        impl Art {
            fn view(&self) -> Value {
                let id = |id: &str, x: i64| json!({"id":id,"rect":[x,0,10,10],"at":[x + 5,5]});
                json!({"screen":"menu","ids":[id("options.light",0),id("belt.potion",10),
                    id("hud.dead",20),id("inventory.slot(0,0)",30)],
                    "digest":self.value,"sent":self.sent})
            }
        }
        let report = walk(&mut Art::default(), FrameSize::D2RUST, 10, 2).unwrap();
        let quiet: Vec<_> = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["rule"] == "no_response")
            .map(|v| v["element"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(quiet, ["click ui:hud.dead"]);
    }
    /// A toggle whose text a hover already shows (the globe text): the
    /// click is told apart once the pointer left. An element the screen
    /// marks idle (the tab already shown) is not a no_response; an
    /// unmarked one that does nothing still is.
    #[test]
    fn no_response_skips_idle_and_looks_past_the_hover() {
        #[derive(Default)]
        struct Globe {
            on: bool,
            pointing: bool,
        }
        impl Driver for Globe {
            fn reset(&mut self, _: &[Action]) -> Result<Value> {
                *self = Globe::default();
                Ok(self.view())
            }
            fn act(&mut self, action: &Action) -> Result<Value> {
                if let Action::Click(id) = action {
                    self.pointing = true;
                    if id == "hud.hp_text" {
                        self.on = !self.on;
                    }
                }
                Ok(self.view())
            }
            fn hover(&mut self, route: &[Action], _: &str) -> Result<Option<Value>> {
                self.reset(route)?;
                self.pointing = true;
                Ok(Some(self.view()))
            }
            fn away(&mut self, route: &[Action], id: &str, click: bool) -> Result<Option<Value>> {
                self.reset(route)?;
                if click {
                    self.act(&Action::Click(id.into()))?;
                }
                self.pointing = false;
                Ok(Some(self.view()))
            }
        }
        impl Globe {
            fn view(&self) -> Value {
                let id = |id: &str, x: i64| json!({"id":id,"rect":[x,0,10,10],"at":[x + 5,5]});
                let mut tab = id("skilltree.tab(1)", 10);
                tab["idle"] = json!("the tab already shown");
                json!({"screen":"game","ids":[id("hud.hp_text",0),tab,id("hud.dead",20)],
                    "digest":self.on || self.pointing})
            }
        }
        let report = walk(&mut Globe::default(), FrameSize::D2RUST, 10, 2).unwrap();
        let quiet: Vec<_> = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["rule"] == "no_response")
            .map(|v| v["element"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(quiet, ["click ui:hud.dead"]);
    }
    #[test]
    fn slider_probes_only_two_ends_and_layout_trips() {
        let ui = json!({"screen":"game","ids":[{"id":"options.volume","rect":[-1,0,20,20],"at":[5,5],"ends":[[0,2],[19,2]],"text_rect":[0,0,30,10]},
            {"id":"close","rect":[0,0,10,10],"at":[99,99]}]});
        assert_eq!(
            actions(&ui)
                .iter()
                .filter(|a| matches!(a, Action::End(..)))
                .count(),
            2
        );
        let rules: Vec<_> = layout(&ui, FrameSize::D2RUST)
            .into_iter()
            .map(|v| v["rule"].as_str().unwrap().to_owned())
            .collect();
        for rule in ["offscreen", "overlap", "unreachable", "text_overflow"] {
            assert!(rules.iter().any(|r| r == rule));
        }
    }
}
