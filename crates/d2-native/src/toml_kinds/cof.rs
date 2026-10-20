//! Animation layouts (`.cof`) as `P.toml`.
//!
//! A COF describes one animation of a composite unit: how many frames and
//! directions it has, its bounding box and speed, which component layers
//! (head, torso, weapon, ...) take part, the event on each frame (attack,
//! missile, sound, ...), and for every direction and frame the order the
//! layers are drawn in, back to front. The sidecar keeps all of it, one
//! `[[layer]]` table per layer and one `draw_order` row per direction.

use d2_formats::cof::{Cof, CofLayer, COMPONENTS};

use super::text::{
    check_round_trip, latin1, not_representable, toml_array, unlatin1, Out, Tab, TextError,
};

/// Writes a COF sidecar. The struct must hold the invariants the parser
/// guarantees: the layer count, frame count and direction count agree with
/// the layer, event and draw-order lists.
pub fn write_cof(file: &str, c: &Cof) -> Result<String, TextError> {
    let (l, f, d) = (
        usize::from(c.layers_count),
        usize::from(c.frames),
        usize::from(c.directions),
    );
    if c.layers.len() != l {
        return Err(not_representable(file, "layers_count differs from layers"));
    }
    if c.events.len() != f {
        return Err(not_representable(file, "events differ from frames"));
    }
    if c.draw_order.len() != d * f * l {
        return Err(not_representable(
            file,
            "draw_order is not directions × frames × layers",
        ));
    }
    let mut o = Out::header("cof");
    o.int("version", c.version);
    o.hex("unknown", u64::from(u32::from_be_bytes(c.unknown)), 8);
    o.int("frames", c.frames);
    o.int("directions", c.directions);
    o.int("x_min", c.x_min);
    o.int("x_max", c.x_max);
    o.int("y_min", c.y_min);
    o.int("y_max", c.y_max);
    o.int("animation_rate", c.animation_rate);
    o.ints("events", c.events.iter());
    o.ints("event_padding", c.event_padding.iter());
    // One row per direction: for each frame, the component IDs of its
    // layers, back to front.
    let row = f * l;
    let rows = (0..d).map(|i| {
        let ids = &c.draw_order[i * row..(i + 1) * row];
        toml_array(ids.iter().map(|b| b.to_string()))
    });
    o.raw("draw_order", toml_array(rows));
    for ly in &c.layers {
        o.table("[[layer]]");
        o.int("component", ly.component);
        o.int("shadow", ly.shadow);
        o.int("selectable", ly.selectable);
        o.int("override_translucency", ly.override_translucency);
        o.int("new_translucency", ly.new_translucency);
        o.str("weapon_class", &latin1(&ly.weapon_class));
    }
    Ok(o.0)
}

/// Reads a COF sidecar back into the decoded struct.
pub fn read_cof(file: &str, text: &str) -> Result<Cof, TextError> {
    let mut t = Tab::parse(file, text, "cof")?;
    let version = t.u8("version")?;
    let unknown = (t.hex("unknown", 8)? as u32).to_be_bytes();
    let frames = t.u8("frames")?;
    let directions = t.u8("directions")?;
    let x_min = t.i32("x_min")?;
    let x_max = t.i32("x_max")?;
    let y_min = t.i32("y_min")?;
    let y_max = t.i32("y_max")?;
    let animation_rate = t.u32("animation_rate")?;
    let events = t.u8_array("events")?;
    let event_padding = t.u8_array("event_padding")?;
    let order_rows = match t.take("draw_order")? {
        toml::Value::Array(a) => a,
        _ => return Err(t.err("`draw_order` is not an array")),
    };
    let mut layers = Vec::new();
    for ly in t.tables("layer")? {
        layers.push(read_layer(file, ly)?);
    }
    let layers_count = u8::try_from(layers.len()).map_err(|_| t.err("more than 255 layers"))?;
    if events.len() != usize::from(frames) {
        return Err(t.err(format!("{} events for {frames} frames", events.len())));
    }
    if order_rows.len() != usize::from(directions) {
        return Err(t.err(format!(
            "draw_order has {} rows for {directions} directions",
            order_rows.len()
        )));
    }
    let row_len = usize::from(frames) * layers.len();
    let draw_order = read_draw_order(&t, order_rows, row_len)?;
    t.finish()?;
    Ok(Cof {
        layers_count,
        frames,
        directions,
        version,
        unknown,
        x_min,
        x_max,
        y_min,
        y_max,
        animation_rate,
        layers,
        events,
        event_padding,
        draw_order,
    })
}

/// One `[[layer]]` table.
fn read_layer(file: &str, mut ly: Tab) -> Result<CofLayer, TextError> {
    let component = ly.u8("component")?;
    if usize::from(component) >= COMPONENTS {
        return Err(ly.err(format!("component {component}")));
    }
    let shadow = ly.u8("shadow")?;
    let selectable = ly.u8("selectable")?;
    let override_translucency = ly.u8("override_translucency")?;
    let new_translucency = ly.u8("new_translucency")?;
    let wc = ly.string("weapon_class")?;
    let wc = unlatin1(file, "weapon_class", &wc)?;
    let weapon_class: [u8; 4] = wc
        .try_into()
        .map_err(|_| ly.err("weapon_class is not 4 characters"))?;
    ly.finish()?;
    Ok(CofLayer {
        component,
        shadow,
        selectable,
        override_translucency,
        new_translucency,
        weapon_class,
    })
}

/// The `draw_order` rows, each `row_len` component IDs long, flattened in
/// direction order.
fn read_draw_order(t: &Tab, rows: Vec<toml::Value>, row_len: usize) -> Result<Vec<u8>, TextError> {
    let mut draw_order = Vec::with_capacity(row_len * rows.len());
    for (i, r) in rows.into_iter().enumerate() {
        let toml::Value::Array(items) = r else {
            return Err(t.err(format!("draw_order row {i} is not an array")));
        };
        if items.len() != row_len {
            return Err(t.err(format!(
                "draw_order row {i} has {} entries, expected {row_len}",
                items.len()
            )));
        }
        for v in items {
            match v {
                toml::Value::Integer(b) if (0..COMPONENTS as i64).contains(&b) => {
                    draw_order.push(b as u8)
                }
                _ => return Err(t.err(format!("draw_order row {i}: not a component ID"))),
            }
        }
    }
    Ok(draw_order)
}

/// Round-trip check for COF: write, read back, compare.
pub fn check_cof(file: &str, original: &Cof) -> Result<(), TextError> {
    check_round_trip(file, original, write_cof, read_cof)
}
