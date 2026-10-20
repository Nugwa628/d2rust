//! The timer-event queue (D2MOO `D2EventTimerQueueStrc`, game +0xB8):
//! per-class buckets keyed on `expire % 64`, per-class every-tick lists,
//! the iteration cursor, and each unit's own timer list.
//!
//! What an event *does* is not here: the runner hands each timer to an
//! [`super::EventDispatch`] (an open question).

use thiserror::Error;

use crate::units::{UnitId, UnitType};

/// Timer buckets per class (`frame % 64`).
pub const BUCKETS: i32 = 64;

/// Event types 0..14; scheduling a type ≥ 15 does nothing.
pub const EVENT_TYPES: u32 = 15;

/// Timer flags (timer +0x02).
pub mod flags {
    /// The callback is running.
    pub const EXECUTING: u16 = 1;
    /// On the free list.
    pub const FREE: u16 = 2;
    /// In an every-tick list.
    pub const EVERY_TICK: u16 = 4;
    /// Cancelled while executing: freed after the callback.
    pub const DELETE: u16 = 8;
}

/// Timer class (table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TimerClass {
    Player = 0,
    Monster = 1,
    Missile = 2,
    Object = 3,
    Item = 4,
}

impl TimerClass {
    /// The order the queue runs the classes in.
    pub const RUN_ORDER: [TimerClass; 5] = [
        TimerClass::Missile,
        TimerClass::Player,
        TimerClass::Monster,
        TimerClass::Object,
        TimerClass::Item,
    ];

    /// Class of a unit type; tiles have none (table entry −1).
    pub const fn of(ty: UnitType) -> Option<TimerClass> {
        match ty {
            UnitType::Player => Some(TimerClass::Player),
            UnitType::Monster => Some(TimerClass::Monster),
            UnitType::Missile => Some(TimerClass::Missile),
            UnitType::Object => Some(TimerClass::Object),
            UnitType::Item => Some(TimerClass::Item),
            UnitType::Tile => None,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

/// A timer record, by slot and generation.
///
/// The slot is the record the base game takes from and pushes back on the
/// slab free list: a freed slot is the next one
/// reused. The generation is D2RUST's own: it changes each time the record
/// is freed, so a handle kept past its timer's free names no timer (a
/// free timer, does nothing for it) instead of the record's
/// next timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimerId {
    slot: u32,
    generation: u32,
}

impl TimerId {
    /// The record slot (reused through the free list).
    pub const fn slot(self) -> u32 {
        self.slot
    }
}

/// A timer's explicit callback (timer +0x2C). Opaque: the
/// [`super::EventDispatch`] implementation gives ids their meaning. A
/// timer without one runs its class's default handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallbackId(pub u32);

/// Which list of a class a timer runs from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimerList {
    /// The class's every-tick list (expire −1).
    EveryTick,
    /// The class's bucket for this frame, due timers only.
    Due,
}

/// The unit a timer belongs to (timer +0x08, +0x0C, +0x10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerOwner {
    pub unit: UnitId,
    pub unit_type: UnitType,
    pub guid: u32,
}

/// Scheduling errors (D2RUST API misuse; the base game has no checks).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimerError {
    /// The unit type has no timer class (tiles). Unit-less timers (type
    /// 6, edge case 3) cannot be expressed: [`TimerOwner`] always has a
    /// unit. TODO(open question 2): model them if a base-game caller
    /// schedules one.
    #[error("unit type {0:?} has no timer class")]
    NoTimerClass(UnitType),
}

/// One execution of a timer, as handed to the dispatcher. Also the record
/// the trace comparison is defined on (class, list,
/// event type, unit type, unit GUID, expire, arg1, arg2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerRun {
    pub timer: TimerId,
    pub class: TimerClass,
    pub list: TimerList,
    pub event: u8,
    pub owner: TimerOwner,
    /// Expire frame; −1 for every-tick timers.
    pub expire: i32,
    pub arg1: u32,
    pub arg2: u32,
    pub callback: Option<CallbackId>,
}

#[derive(Clone, Debug)]
struct Timer {
    event: u8,
    flags: u16,
    expire: i32,
    owner: TimerOwner,
    class: TimerClass,
    arg1: u32,
    arg2: u32,
    /// Bucket / every-tick list links (+0x1C, +0x20).
    next: Option<TimerId>,
    prev: Option<TimerId>,
    /// Unit timer list links (+0x24, +0x28).
    unit_next: Option<TimerId>,
    unit_prev: Option<TimerId>,
    /// Still linked in its unit's timer list (cleared when the unit is
    /// removed while this timer executes).
    in_unit_list: bool,
    callback: Option<CallbackId>,
    /// D2RUST handle generation of this record (see [`TimerId`]).
    generation: u32,
}

/// Signed frame remainder (`idiv`): negative for negative frames.
pub const fn frame_mod(frame: i32, n: i32) -> i32 {
    frame % n
}

/// Bucket array index of a frame or expire value.
///
/// TODO(handoff open question T1): a frame past 2^31 − 1 gives a negative
/// bucket, which in the base game addresses neighbouring queue fields
/// (layout). Not modelled: unreachable in practice (≈ 994 days).
fn bucket_slot(v: i32) -> usize {
    let b = frame_mod(v, BUCKETS);
    assert!(
        b >= 0,
        "negative timer bucket {b} (frame past 2^31 - 1, tick.md §2.2): not modelled"
    );
    b as usize
}

/// The timer queue.
#[derive(Clone, Debug)]
pub struct TimerQueue {
    /// Current bucket index (+0x000).
    bucket: i32,
    heads: [[Option<TimerId>; BUCKETS as usize]; 5],
    tails: [[Option<TimerId>; BUCKETS as usize]; 5],
    every_tick: [Option<TimerId>; 5],
    /// Iteration cursor (+0xA18): next timer to visit.
    cursor: Option<TimerId>,
    timers: Vec<Timer>,
    /// Slab free list (slots).
    free: Vec<u32>,
    /// Each unit's timer list head, by unit slot.
    unit_heads: Vec<Option<TimerId>>,
}

impl Default for TimerQueue {
    fn default() -> Self {
        Self {
            bucket: 0,
            heads: [[None; BUCKETS as usize]; 5],
            tails: [[None; BUCKETS as usize]; 5],
            every_tick: [None; 5],
            cursor: None,
            timers: Vec::new(),
            free: Vec::new(),
            unit_heads: Vec::new(),
        }
    }
}

impl TimerQueue {
    pub fn new() -> Self {
        Self::default()
    }

    fn t(&mut self, id: TimerId) -> &mut Timer {
        &mut self.timers[id.slot as usize]
    }

    fn live(&self, id: TimerId) -> Option<&Timer> {
        self.timers
            .get(id.slot as usize)
            .filter(|t| t.generation == id.generation && t.flags & flags::FREE == 0)
    }

    /// The current bucket index (queue +0x000), set by each run.
    pub fn current_bucket(&self) -> i32 {
        self.bucket
    }

    /// A live timer's expire frame (−1 for every-tick timers).
    pub fn expire(&self, id: TimerId) -> Option<i32> {
        self.live(id).map(|t| t.expire)
    }

    /// A live timer's flags.
    pub fn flags(&self, id: TimerId) -> Option<u16> {
        self.live(id).map(|t| t.flags)
    }

    /// A live timer's event type and arguments.
    pub fn event(&self, id: TimerId) -> Option<(u8, u32, u32)> {
        self.live(id).map(|t| (t.event, t.arg1, t.arg2))
    }

    fn unit_head(&mut self, unit: UnitId) -> &mut Option<TimerId> {
        let i = unit.0 as usize;
        if self.unit_heads.len() <= i {
            self.unit_heads.resize(i + 1, None);
        }
        &mut self.unit_heads[i]
    }

    /// A unit's timers, head first (newest scheduled first).
    pub fn unit_timers(&self, unit: UnitId) -> Vec<TimerId> {
        let mut out = Vec::new();
        let mut cur = self.unit_heads.get(unit.0 as usize).copied().flatten();
        while let Some(t) = cur {
            out.push(t);
            cur = self.timers[t.slot as usize].unit_next;
        }
        out
    }

    /// Bucket `b` (0..63) of `class`, head first.
    pub fn bucket(&self, class: TimerClass, b: usize) -> Vec<TimerId> {
        self.walk(self.heads[class.index()][b])
    }

    /// The every-tick list of `class`, head first.
    pub fn every_tick(&self, class: TimerClass) -> Vec<TimerId> {
        self.walk(self.every_tick[class.index()])
    }

    fn walk(&self, mut cur: Option<TimerId>) -> Vec<TimerId> {
        let mut out = Vec::new();
        while let Some(t) = cur {
            out.push(t);
            cur = self.timers[t.slot as usize].next;
        }
        out
    }

    /// Takes a timer from the free list and links it at the head of its
    /// unit's timer list.
    #[allow(clippy::too_many_arguments)]
    fn alloc(
        &mut self,
        owner: TimerOwner,
        class: TimerClass,
        event: u8,
        flags: u16,
        expire: i32,
        callback: Option<CallbackId>,
        arg1: u32,
        arg2: u32,
    ) -> TimerId {
        let mut timer = Timer {
            event,
            flags,
            expire,
            owner,
            class,
            arg1,
            arg2,
            next: None,
            prev: None,
            unit_next: None,
            unit_prev: None,
            in_unit_list: true,
            callback,
            generation: 0,
        };
        let id = match self.free.pop() {
            Some(slot) => {
                let generation = self.timers[slot as usize].generation;
                timer.generation = generation;
                self.timers[slot as usize] = timer;
                TimerId { slot, generation }
            }
            None => {
                self.timers.push(timer);
                TimerId {
                    slot: (self.timers.len() - 1) as u32,
                    generation: 0,
                }
            }
        };
        let head = self.unit_head(owner.unit).replace(id);
        self.t(id).unit_next = head;
        if let Some(h) = head {
            self.t(h).unit_prev = Some(id);
        }
        id
    }

    /// Schedule a timed event at `frame` (the game's
    /// current frame). `expire == -1` schedules an every-tick event
    /// with a **null callback** (pushes 0, so
    /// the caller's callback is lost and the class default handler runs).
    /// Returns `None` for event types ≥ 15.
    ///
    /// Precondition: for a monster's AI-think event (type 2)
    /// with an expire other than −1, the caller first runs the monster
    /// state-54 check (with (unit, 0) when the monster has
    /// `STATE_UNINTERRUPTABLE`); [`needs_uninterruptable_check`] tells
    /// when. TODO(monsters): move the check here once monster states
    /// exist.
    #[allow(clippy::too_many_arguments)]
    pub fn schedule(
        &mut self,
        frame: i32,
        owner: TimerOwner,
        event: u32,
        expire: i32,
        callback: Option<CallbackId>,
        arg1: u32,
        arg2: u32,
    ) -> Result<Option<TimerId>, TimerError> {
        if event >= EVENT_TYPES {
            return Ok(None);
        }
        if expire == -1 {
            return self.schedule_every_tick(owner, event, None, arg1, arg2);
        }
        let class =
            TimerClass::of(owner.unit_type).ok_or(TimerError::NoTimerClass(owner.unit_type))?;
        let expire = if expire <= frame {
            frame.wrapping_add(1)
        } else {
            expire
        };
        let b = bucket_slot(expire);
        let id = self.alloc(owner, class, event as u8, 0, expire, callback, arg1, arg2);
        let c = class.index();
        let tail = self.tails[c][b].replace(id);
        self.t(id).prev = tail;
        match tail {
            Some(t) => self.t(t).next = Some(id),
            None => self.heads[c][b] = Some(id),
        }
        Ok(Some(id))
    }

    /// Schedule an every-tick event, prepended to its
    /// class's every-tick list. Returns `None` for event types ≥ 15.
    pub fn schedule_every_tick(
        &mut self,
        owner: TimerOwner,
        event: u32,
        callback: Option<CallbackId>,
        arg1: u32,
        arg2: u32,
    ) -> Result<Option<TimerId>, TimerError> {
        if event >= EVENT_TYPES {
            return Ok(None);
        }
        let class =
            TimerClass::of(owner.unit_type).ok_or(TimerError::NoTimerClass(owner.unit_type))?;
        let id = self.alloc(
            owner,
            class,
            event as u8,
            flags::EVERY_TICK,
            -1,
            callback,
            arg1,
            arg2,
        );
        let head = self.every_tick[class.index()].replace(id);
        self.t(id).next = head;
        if let Some(h) = head {
            self.t(h).prev = Some(id);
        }
        Ok(Some(id))
    }

    /// Cancel a timer. An executing timer is only
    /// marked (flag 8) and freed by the runner after its callback.
    /// Cancelling a free timer does nothing.
    pub fn cancel(&mut self, id: TimerId) {
        let Some(t) = self.live(id) else {
            return;
        };
        if t.flags & flags::EXECUTING != 0 {
            self.t(id).flags |= flags::DELETE;
            return;
        }
        let t = t.clone();
        if self.cursor == Some(id) {
            self.cursor = t.next;
        }
        let c = t.class.index();
        let every_tick = t.flags & flags::EVERY_TICK != 0;
        match t.prev {
            Some(p) => self.t(p).next = t.next,
            None if every_tick => self.every_tick[c] = t.next,
            None => self.heads[c][bucket_slot(t.expire)] = t.next,
        }
        match t.next {
            Some(n) => self.t(n).prev = t.prev,
            None if every_tick => {}
            None => self.tails[c][bucket_slot(t.expire)] = t.prev,
        }
        if t.in_unit_list {
            match t.unit_prev {
                Some(p) => self.t(p).unit_next = t.unit_next,
                None => *self.unit_head(t.owner.unit) = t.unit_next,
            }
            if let Some(n) = t.unit_next {
                self.t(n).unit_prev = t.unit_prev;
            }
        }
        let t = self.t(id);
        t.flags = flags::FREE;
        t.next = None;
        t.prev = None;
        t.unit_next = None;
        t.unit_prev = None;
        t.in_unit_list = false;
        t.generation = t.generation.wrapping_add(1);
        self.free.push(id.slot);
    }

    /// Cancels the unit's timers matching `keep`, walking its timer list
    /// from the head with the next link saved first.
    fn cancel_unit_where(&mut self, unit: UnitId, pred: impl Fn(&Timer) -> bool) {
        let mut cur = self.unit_heads.get(unit.0 as usize).copied().flatten();
        while let Some(id) = cur {
            let t = &self.timers[id.slot as usize];
            cur = t.unit_next;
            if pred(t) {
                self.cancel(id);
            }
        }
    }

    /// Cancels the unit's events of type `event`, and with `arg1` when
    /// given (uses it with "any arg" and with a skill
    /// id).
    pub fn cancel_unit_events(&mut self, unit: UnitId, event: u8, arg1: Option<u32>) {
        self.cancel_unit_where(unit, |t| {
            t.event == event && arg1.is_none_or(|a| t.arg1 == a)
        });
    }

    /// Cancels the unit's events of type `event` with `callback`.
    pub fn cancel_unit_events_with_callback(
        &mut self,
        unit: UnitId,
        event: u8,
        callback: Option<CallbackId>,
    ) {
        self.cancel_unit_where(unit, |t| t.event == event && t.callback == callback);
    }

    /// Cancels all the unit's timers.
    pub fn cancel_unit_timers(&mut self, unit: UnitId) {
        self.cancel_unit_where(unit, |_| true);
    }

    /// Unit removal: cancel all its timers, then
    /// detach the ones still executing from the freed unit's timer list,
    /// so the slot can be reused. The executing timer is freed by the
    /// runner after its callback (consequence 3).
    pub fn remove_unit(&mut self, unit: UnitId) {
        self.cancel_unit_timers(unit);
        let mut cur = self.unit_head(unit).take();
        while let Some(id) = cur {
            let t = self.t(id);
            cur = t.unit_next.take();
            t.unit_prev = None;
            t.in_unit_list = false;
        }
    }

    /// Starts a queue run at `frame`: stores the bucket.
    pub(crate) fn begin_run(&mut self, frame: i32) {
        self.bucket = frame_mod(frame, BUCKETS);
    }

    /// Points the cursor at the first timer of a list.
    pub(crate) fn begin_list(&mut self, class: TimerClass, list: TimerList) {
        let c = class.index();
        self.cursor = match list {
            TimerList::EveryTick => self.every_tick[c],
            TimerList::Due => self.heads[c][bucket_slot(self.bucket)],
        };
    }

    /// Advances the cursor to the next timer to run in the current list
    /// and marks it executing. Due lists skip timers whose
    /// expire is not `frame`.
    pub(crate) fn next_run(&mut self, list: TimerList, frame: i32) -> Option<TimerRun> {
        loop {
            let id = self.cursor?;
            let t = &self.timers[id.slot as usize];
            self.cursor = t.next;
            if list == TimerList::Due && t.expire != frame {
                continue;
            }
            let run = TimerRun {
                timer: id,
                class: t.class,
                list,
                event: t.event,
                owner: t.owner,
                expire: t.expire,
                arg1: t.arg1,
                arg2: t.arg2,
                callback: t.callback,
            };
            self.t(id).flags |= flags::EXECUTING;
            return Some(run);
        }
    }

    /// After a callback: clear the executing flag; free a
    /// bucket timer always, an every-tick timer only if it was cancelled
    /// during the call.
    pub(crate) fn finish_run(&mut self, run: &TimerRun) {
        let t = self.t(run.timer);
        t.flags &= !flags::EXECUTING;
        if run.list == TimerList::Due || t.flags & flags::DELETE != 0 {
            self.cancel(run.timer);
        }
    }
}

/// Whether applies before scheduling: a monster's AI-think
/// event (type 2) on the timed path. The state-54 test itself belongs to
/// the monsters.
pub const fn needs_uninterruptable_check(unit_type: UnitType, event: u32, expire: i32) -> bool {
    matches!(unit_type, UnitType::Monster) && event == 2 && expire != -1
}
