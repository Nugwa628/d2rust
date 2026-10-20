// D2RUST's own: dev tools.
//! The clickable dev panel above the console. Every button sends a
//! console line (the same commands, `d2_proto::dev::parse`); the panel
//! keeps only what it shows (act tab, picker search, quality, counts),
//! never game state. The content is rebuilt from [`PanelState`] when it
//! or the switch states change.

use bevy::prelude::*;
use d2_proto::dev::{DevSwitchState, Quality};

use super::server::DevCatalog;

/// Picker entries shown.
pub const PICKS: usize = 14;
/// Speed presets (percent).
pub const SPEEDS: [u32; 4] = [50, 100, 200, 400];

/// Which text field takes typed keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Field {
    #[default]
    Console,
    Search,
    Level,
    Slot,
}

/// What the picker lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pick {
    #[default]
    Monsters,
    Items,
}

/// What a button does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelAction {
    /// Send this console line.
    Run(String),
    Act(u8),
    Picker(Pick),
    Focus(Field),
    Quality(Quality),
    Ilvl(i32),
    Count(i32),
    Ground,
    /// Select a picker entry (monster id, or item code).
    Select(String),
    /// Spawn the selected monster / create the selected item.
    Make,
    /// `level` with the level field.
    SetLevel,
    /// `save` / `load` with the slot field.
    SaveSlot,
    LoadSlot,
}

/// The panel's own state (what it shows; never game state).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelState {
    pub act: u8,
    pub pick: Pick,
    pub search: String,
    pub level: String,
    pub quality: Quality,
    pub ilvl: u8,
    pub count: u8,
    pub ground: bool,
    pub focus: Field,
    pub selected: Option<String>,
    pub slot: String,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            act: 1,
            pick: Pick::Monsters,
            search: String::new(),
            level: String::new(),
            quality: Quality::Normal,
            ilvl: 30,
            count: 1,
            ground: false,
            focus: Field::Console,
            selected: None,
            slot: String::new(),
        }
    }
}

impl PanelState {
    /// Applies a button; a console line to send when it has one.
    pub fn press(&mut self, a: &PanelAction) -> Option<String> {
        match a {
            PanelAction::Run(line) => return Some(line.clone()),
            PanelAction::Act(n) => self.act = *n,
            PanelAction::Picker(p) => {
                if self.pick != *p {
                    self.pick = *p;
                    self.selected = None;
                    self.search.clear();
                }
            }
            PanelAction::Focus(f) => self.focus = *f,
            PanelAction::Quality(q) => self.quality = *q,
            PanelAction::Ilvl(d) => self.ilvl = (i32::from(self.ilvl) + d).clamp(1, 99) as u8,
            PanelAction::Count(d) => self.count = (i32::from(self.count) + d).clamp(1, 20) as u8,
            PanelAction::Ground => self.ground = !self.ground,
            PanelAction::Select(s) => self.selected = Some(s.clone()),
            PanelAction::Make => {
                let s = self.selected.as_ref()?;
                return Some(match self.pick {
                    Pick::Monsters => format!("spawn {s} {}", self.count),
                    Pick::Items => format!(
                        "item {s} {} ilvl {}{}",
                        self.quality.name(),
                        self.ilvl,
                        if self.ground { " ground" } else { "" }
                    ),
                });
            }
            PanelAction::SaveSlot | PanelAction::LoadSlot => {
                let n = self.slot.trim();
                let kw = if *a == PanelAction::SaveSlot {
                    "save"
                } else {
                    "load"
                };
                return Some(if n.is_empty() {
                    kw.to_string()
                } else {
                    format!("{kw} {n}")
                });
            }
            PanelAction::SetLevel => {
                let n = self.level.trim().to_string();
                self.level.clear();
                return (!n.is_empty()).then(|| format!("level {n}"));
            }
        }
        None
    }

    /// The typed character goes to the focused field (not the console).
    pub fn type_char(&mut self, c: char) {
        match self.focus {
            Field::Search if !c.is_control() && self.search.len() < 30 => self.search.push(c),
            Field::Level if (c.is_ascii_digit() || c == '+') && self.level.len() < 3 => {
                self.level.push(c)
            }
            Field::Slot
                if (c.is_ascii_alphanumeric() || c == '-' || c == '_') && self.slot.len() < 32 =>
            {
                self.slot.push(c)
            }
            _ => {}
        }
    }

    pub fn backspace(&mut self) {
        match self.focus {
            Field::Search => {
                self.search.pop();
            }
            Field::Level => {
                self.level.pop();
            }
            Field::Slot => {
                self.slot.pop();
            }
            Field::Console => {}
        }
    }
}

/// The picker's entries for the search: (label, the id or code it
/// selects), at most [`PICKS`].
pub fn picks(cat: &DevCatalog, pick: Pick, search: &str) -> Vec<(String, String)> {
    let s = search.to_ascii_lowercase();
    let keep = |name: &str, key: &str| {
        !name.is_empty()
            && (s.is_empty()
                || name.to_ascii_lowercase().contains(&s)
                || key.eq_ignore_ascii_case(&s))
    };
    match pick {
        Pick::Monsters => cat
            .monsters
            .iter()
            .filter(|(id, n)| keep(n, &id.to_string()))
            .take(PICKS)
            .map(|(id, n)| (format!("{n} ({id})"), id.to_string()))
            .collect(),
        Pick::Items => cat
            .items
            .iter()
            .filter(|(c, n)| keep(n, c))
            .take(PICKS)
            .map(|(c, n)| (format!("{n} [{c}]"), c.clone()))
            .collect(),
    }
}

/// A button's action.
#[derive(Component, Clone, Debug)]
pub struct DevButton(pub PanelAction);

const FONT: f32 = 12.0;

fn on_color(on: bool) -> Color {
    if on {
        Color::srgb(0.15, 0.45, 0.15)
    } else {
        Color::srgb(0.22, 0.22, 0.26)
    }
}

fn button(p: &mut ChildSpawnerCommands, label: &str, action: PanelAction, on: bool) {
    p.spawn((
        Button,
        DevButton(action),
        Node {
            padding: UiRect::axes(Val::Px(5.0), Val::Px(2.0)),
            margin: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(on_color(on)),
    ))
    .with_children(|b| {
        b.spawn((
            Text::new(label),
            TextFont {
                font_size: bevy::text::FontSize::Px(FONT),
                ..default()
            },
            TextColor(Color::WHITE),
        ));
    });
}

fn label(p: &mut ChildSpawnerCommands, text: &str) {
    p.spawn((
        Text::new(text),
        TextFont {
            font_size: bevy::text::FontSize::Px(FONT),
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.8, 0.5)),
        Node {
            margin: UiRect::new(Val::Px(1.0), Val::Px(4.0), Val::Px(3.0), Val::Px(1.0)),
            ..default()
        },
    ));
}

fn row(p: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(f);
}

fn column(p: &mut ChildSpawnerCommands, pct: f32, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        width: Val::Percent(pct),
        padding: UiRect::all(Val::Px(3.0)),
        ..default()
    })
    .with_children(f);
}

/// A text field: a button that takes the focus, showing its text.
fn field(p: &mut ChildSpawnerCommands, f: Field, text: &str, focus: Field, hint: &str) {
    let shown = if text.is_empty() && focus != f {
        hint.to_string()
    } else if focus == f {
        format!("{text}_")
    } else {
        text.to_string()
    };
    button(p, &shown, PanelAction::Focus(f), focus == f);
}

/// The left column's buttons.
pub fn spawn_left(
    p: &mut ChildSpawnerCommands,
    st: &PanelState,
    sw: DevSwitchState,
    _cat: &DevCatalog,
) {
    let run = |s: &str| PanelAction::Run(s.to_string());
    column(p, 100.0, |c| {
        label(c, "Player");
        row(c, |r| {
            button(
                r,
                if sw.god { "God ON" } else { "God off" },
                run("god"),
                sw.god,
            );
            button(r, "Heal", run("heal"), false);
            button(
                r,
                if sw.nocd {
                    "No cooldown ON"
                } else {
                    "No cooldown off"
                },
                run("nocd"),
                sw.nocd,
            );
            button(
                r,
                if sw.freemana {
                    "Free mana ON"
                } else {
                    "Free mana off"
                },
                run("freemana"),
                sw.freemana,
            );
        });
        label(c, "Speed");
        row(c, |r| {
            button(r, "off", run("speed 0"), sw.speed == 0);
            for s in SPEEDS {
                button(
                    r,
                    &format!("+{s}%"),
                    run(&format!("speed {s}")),
                    sw.speed == s,
                );
            }
        });
        label(c, "Level");
        row(c, |r| {
            button(r, "+1", run("level +1"), false);
            button(r, "+10", run("level +10"), false);
            field(r, Field::Level, &st.level, st.focus, "set...");
            button(r, "Set", PanelAction::SetLevel, false);
        });
        label(c, "Resources");
        row(c, |r| {
            button(r, "Gold +100k", run("gold 100000"), false);
            button(r, "Stat pts +10", run("statpts 10"), false);
            button(r, "Skill pts +10", run("skillpts 10"), false);
        });
        label(c, "Unlock");
        row(c, |r| {
            button(r, "All waypoints", run("waypoints"), false);
            button(r, "All acts", run("acts"), false);
            button(r, "Identify all", run("identify"), false);
        });
        label(c, "Tools");
        row(c, |r| {
            button(r, "Info overlay", run("info"), false);
            button(r, "Bug report", run("bugreport"), false);
            button(r, "Save slot", PanelAction::SaveSlot, false);
            field(r, Field::Slot, &st.slot, st.focus, "slot name...");
            button(r, "Load slot", PanelAction::LoadSlot, false);
        });
        label(c, "Monsters");
        row(c, |r| {
            button(r, "Kill screen", run("kill screen"), false);
            button(r, "Kill level", run("kill level"), false);
        });
        label(c, "Town");
        row(c, |r| {
            for a in 1..=5u8 {
                button(r, &format!("Act {a}"), run(&format!("town {a}")), false);
            }
        });
    });
}

/// The warp list and the picker.
pub fn spawn_right(
    p: &mut ChildSpawnerCommands,
    st: &PanelState,
    _sw: DevSwitchState,
    cat: &DevCatalog,
) {
    let run = |s: &str| PanelAction::Run(s.to_string());
    column(p, 50.0, |c| {
        label(c, "Warp");
        row(c, |r| {
            for a in 1..=5u8 {
                button(r, &format!("Act {a}"), PanelAction::Act(a), st.act == a);
            }
        });
        row(c, |r| {
            for ((id, name), act) in cat.levels.iter().zip(&cat.level_acts) {
                if *act + 1 == st.act && !name.is_empty() {
                    button(r, name, run(&format!("warp {id}")), false);
                }
            }
        });
    });
    column(p, 35.0, |c| {
        row(c, |r| {
            button(
                r,
                "Monsters",
                PanelAction::Picker(Pick::Monsters),
                st.pick == Pick::Monsters,
            );
            button(
                r,
                "Items",
                PanelAction::Picker(Pick::Items),
                st.pick == Pick::Items,
            );
            field(r, Field::Search, &st.search, st.focus, "search...");
        });
        match st.pick {
            Pick::Monsters => row(c, |r| {
                button(r, "-", PanelAction::Count(-1), false);
                label(r, &format!("count {}", st.count));
                button(r, "+", PanelAction::Count(1), false);
                button(r, "Spawn", PanelAction::Make, st.selected.is_some());
            }),
            Pick::Items => {
                row(c, |r| {
                    for (n, q) in Quality::ALL {
                        button(r, n, PanelAction::Quality(q), st.quality == q);
                    }
                });
                row(c, |r| {
                    button(r, "-10", PanelAction::Ilvl(-10), false);
                    button(r, "-", PanelAction::Ilvl(-1), false);
                    label(r, &format!("ilvl {}", st.ilvl));
                    button(r, "+", PanelAction::Ilvl(1), false);
                    button(r, "+10", PanelAction::Ilvl(10), false);
                    button(
                        r,
                        if st.ground {
                            "to ground"
                        } else {
                            "to inventory"
                        },
                        PanelAction::Ground,
                        false,
                    );
                    button(r, "Create", PanelAction::Make, st.selected.is_some());
                });
            }
        }
        row(c, |r| {
            for (text, key) in picks(cat, st.pick, &st.search) {
                let on = st.selected.as_deref() == Some(key.as_str());
                button(r, &text, PanelAction::Select(key), on);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> DevCatalog {
        DevCatalog {
            levels: vec![(1, "Rogue Encampment".into()), (40, "Lut Gholein".into())],
            level_acts: vec![0, 1],
            items: vec![
                ("hax".into(), "Hand Axe".into()),
                ("rin".into(), "Ring".into()),
            ],
            monsters: vec![
                (0, "Zombie".into()),
                (19, "Fallen".into()),
                (20, String::new()),
            ],
        }
    }

    #[test]
    fn buttons_make_console_lines() {
        let mut s = PanelState::default();
        assert_eq!(s.press(&PanelAction::Run("god".into())), Some("god".into()));
        assert_eq!(s.press(&PanelAction::Make), None, "nothing selected");
        s.press(&PanelAction::Select("19".into()));
        s.press(&PanelAction::Count(2));
        assert_eq!(s.press(&PanelAction::Make), Some("spawn 19 3".into()));
        s.press(&PanelAction::Picker(Pick::Items));
        assert_eq!(s.selected, None);
        s.press(&PanelAction::Select("rin".into()));
        s.press(&PanelAction::Quality(Quality::Unique));
        s.press(&PanelAction::Ilvl(100));
        s.press(&PanelAction::Ground);
        assert_eq!(
            s.press(&PanelAction::Make),
            Some("item rin unique ilvl 99 ground".into())
        );
        s.focus = Field::Level;
        for c in "4x2".chars() {
            s.type_char(c);
        }
        assert_eq!(s.press(&PanelAction::SetLevel), Some("level 42".into()));
        assert_eq!(s.press(&PanelAction::SetLevel), None);
        s.focus = Field::Slot;
        for c in "a/1".chars() {
            s.type_char(c);
        }
        assert_eq!(s.press(&PanelAction::SaveSlot), Some("save a1".into()));
        assert_eq!(s.press(&PanelAction::LoadSlot), Some("load a1".into()));
        // Every line a button makes parses.
        for l in [
            "spawn 19 3",
            "item rin unique ilvl 99 ground",
            "level 42",
            "level +10",
        ] {
            assert!(d2_proto::dev::parse(l).is_ok(), "{l}");
        }
    }

    #[test]
    fn picker_filters_by_name_or_code() {
        let c = cat();
        assert_eq!(
            picks(&c, Pick::Monsters, "").len(),
            2,
            "unnamed rows hidden"
        );
        assert_eq!(
            picks(&c, Pick::Monsters, "fall"),
            vec![("Fallen (19)".to_string(), "19".to_string())]
        );
        assert_eq!(picks(&c, Pick::Items, "RIN")[0].1, "rin");
        assert_eq!(picks(&c, Pick::Items, "axe")[0].0, "Hand Axe [hax]");
    }
}
