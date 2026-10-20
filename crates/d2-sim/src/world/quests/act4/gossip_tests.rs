// Spec: specs/world/quests-act4.md §6
//! The Act IV gossip records (Tyrael chain 21, Hadriel chain 29) on the
//! quests' fake world.

use super::super::super::tests::*;
use super::super::super::*;
use super::*;

const TYRAEL_U: UnitId = UnitId(0x20);
const HADRIEL_U: UnitId = UnitId(0x24);

fn fake() -> Fake {
    let mut f = Fake::new();
    f.p(P1).act = Some(3);
    for (u, class) in [(TYRAEL_U, npc::TYRAEL2), (HADRIEL_U, HADRIEL)] {
        let kind = UnitKind::Monster {
            class: u32::from(class),
            superunique: None,
            owner: None,
        };
        f.monsters.insert(u, (u.0, class, kind));
    }
    f
}

fn chat(ctl: &mut QuestControl, f: &mut Fake, n: UnitId) -> TextList {
    let mut list = TextList::new();
    ctl.npc_activate(f, P1, n, &mut list);
    list
}

// Covers: specs/world/quests-act4.md §6.1, §6.3
#[test]
fn tyrael_gossip() {
    let (mut ctl, _) = control();
    let mut f = fake();
    let list = chat(&mut ctl, &mut f, TYRAEL_U);
    assert!(list.contains(&(664, 0)));
    // 664 from Hadriel: nothing; from Tyrael: 24.0.
    let mut m = |f: &mut Fake, n: UnitId| {
        let mut b = vec![0x31];
        b.extend(f.guid(n).to_le_bytes());
        b.extend(664u16.to_le_bytes());
        b.extend([0, 0]);
        ctl.quest_message(f, P1, &b);
    };
    m(&mut f, HADRIEL_U);
    assert!(!f.flags(P1).get(24, 0));
    m(&mut f, TYRAEL_U);
    assert!(f.flags(P1).get(24, 0));
    let (mut ctl, _) = control();
    assert!(!chat(&mut ctl, &mut f, TYRAEL_U).contains(&(664, 0)));
    // Active / status.
    let i = ctl.find(21).unwrap();
    assert!(!active(&ctl, &mut f, i, P1, npc::TYRAEL2, 0x005B_3750));
    f.p(P1).quests.flags[0].clear(24, 0);
    assert!(active(&ctl, &mut f, i, P1, npc::TYRAEL2, 0x005B_3750));
    assert!(!active(&ctl, &mut f, i, P1, HADRIEL, 0x005B_3750));
    let pf = f.flags(P1);
    assert_eq!(status(&ctl, &mut f, i, P1, &pf, 0x005B_3740), None);
    // No status function report: 0x52 carries 0 for slot 24.
    ctl.record_mut(21).unwrap().status = 3;
    ctl.request_quest_data(&mut f, P1).unwrap();
    let m52 = &f.sent.last().unwrap().1;
    assert_eq!((m52[0], m52[1 + 24]), (0x52, 0));
    assert!(!f.log.iter().any(|l| l.starts_with("unhandled 21")));
}

// Covers: specs/world/quests-act4.md §6.2, §edge-cases-original-bugs r17
#[test]
fn hadriel_gossip() {
    let (mut ctl, _) = control();
    let mut f = fake();
    // Hell's Forge untouched: 668.
    assert_eq!(chat(&mut ctl, &mut f, HADRIEL_U), [(668, 0)]);
    // Forge done, Diablo not killed in this game: 669.
    f.p(P1).quests.flags[0].set(27, 0);
    assert_eq!(chat(&mut ctl, &mut f, HADRIEL_U), [(669, 0)]);
    // Diablo killed in this game (chain 23 +0x14): nothing.
    ctl.record_mut(23).unwrap().extra.a4.q2.killed = true;
    assert_eq!(chat(&mut ctl, &mut f, HADRIEL_U), []);
    // Chain 23 absent: 669.
    ctl.records.retain(|r| r.chain != 23);
    assert_eq!(chat(&mut ctl, &mut f, HADRIEL_U), [(669, 0)]);
    // Wants to talk: forge bits clear, or 26.13 and 26.0 clear.
    let (ctl, _) = control();
    let i = ctl.find(29).unwrap();
    let act = |f: &mut Fake| active(&ctl, f, i, P1, HADRIEL, 0x005B_69F0);
    assert!(act(&mut f)); // 27.0 set, 26.x clear
    f.p(P1).quests.flags[0].set(26, 13);
    assert!(!act(&mut f));
    f.p(P1).quests.flags[0].clear(27, 0);
    assert!(act(&mut f));
    f.p(P1).quests.flags[0].set(27, 13);
    assert!(!act(&mut f));
    assert!(!active(&ctl, &mut f, i, P1, npc::TYRAEL2, 0x005B_69F0));
    let pf = f.flags(P1);
    assert_eq!(status(&ctl, &mut f, i, P1, &pf, 0x005B_69E0), None);
    // No event 11: a scroll message is not dispatched to chain 29.
    let (mut ctl, _) = control();
    let mut b = vec![0x31];
    b.extend(f.guid(HADRIEL_U).to_le_bytes());
    b.extend(668u16.to_le_bytes());
    b.extend([0, 0]);
    let before = f.flags(P1);
    ctl.quest_message(&mut f, P1, &b);
    assert_eq!(f.flags(P1), before);
    assert!(!f.log.iter().any(|l| l.starts_with("unhandled 29")));
}
