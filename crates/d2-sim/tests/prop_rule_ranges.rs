//! Bounds derived from combat::pct/scale and items::inventory::levelreq.
//! Restrict arithmetic to the non-wrapping domain; wrapping is a stated rule.
use d2_sim::{
    combat::{pct, scale},
    items::{
        affixes::alvl,
        inventory::levelreq::{level_requirement, AffixReq, LevelReqItem, CRAFTED_CAP},
    },
};
use proptest::prelude::*;
proptest! {
    #[test]
    fn damage_scaling_is_bounded_and_monotonic(damage in 0i32..=100_000, factor in 0i32..=128, extra in 0i32..=128) {
        let scaled = scale(damage, factor);
        prop_assert!((0..=damage).contains(&scaled), "damage={damage}, factor={factor}, actual={scaled}");
        prop_assert!(scale(damage, factor.max(extra)) >= scaled);
        prop_assert_eq!(pct(damage, factor, 128), scaled);
        prop_assert_eq!(scale(damage, 128), damage);
    }
    #[test]
    fn affix_level_bounded_and_monotonic(ilvl in 1i32..=99, qlvl in 1i32..=99, magic in 0i32..=20, delta in 0i32..=99) {
        let a = alvl(ilvl, qlvl, magic);
        prop_assert!((1..=99).contains(&a), "alvl={a}");
        prop_assert!(alvl((ilvl+delta).min(99), qlvl, magic) >= a);
    }
    #[test]
    fn generated_affix_requirements_obey_quality_rules(reqs in prop::collection::vec(0i32..=99, 0..=6), base in 0i32..=99, offset in -99i32..=99) {
        let mut item = LevelReqItem {quality:8, class_levelreq:base, stat_levelreq:offset, ..Default::default()};
        for (slot, &req) in item.prefixes.iter_mut().chain(item.suffixes.iter_mut()).zip(&reqs) {
            *slot=Some(AffixReq{levelreq:req, class:0xFF, classlevelreq:0});
        }
        let maximum = reqs.iter().copied().max().unwrap_or(0);
        let crafted = (maximum+10+3*reqs.len() as i32).min(CRAFTED_CAP);
        prop_assert_eq!(level_requirement(&item,None), (crafted.max(base)+offset).max(0));
        item.quality=6;
        let before=level_requirement(&item,None);
        prop_assert_eq!(before, (maximum.max(base)+offset).max(0));
        item.class_levelreq+=1;
        prop_assert!(level_requirement(&item,None)>=before);
    }
}
