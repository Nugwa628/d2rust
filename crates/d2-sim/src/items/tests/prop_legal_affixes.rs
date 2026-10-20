//! The roller must select a table row passing its level/type/spawn filters.
use super::*;
use crate::items::affixes::{affix, alvl, magic_fits, roll_affix};
use proptest::prelude::*;
proptest! {
    #[test]
    fn rolled_affixes_are_legal(seed in any::<u32>(), ilvl in 1i32..=99,
        rows in prop::collection::vec((1i32..=99, any::<bool>(), 0u8..=10), 1..24)) {
        let mut t=tables();
        let record=push_item(&mut t,item_rec(RING,b"rin "));
        t.n_suffix=rows.len();
        t.magic=rows.iter().enumerate().map(|(i,&(level,spawn,freq))| {
            let mut row=affix_row(RING,i as i32+1);
            row.level=level;
            row.spawnable=u8::from(spawn);
            row.frequency=freq;
            row
        }).collect();
        let mut it=item(record,seed);
        it.ilvl=ilvl;
        let id=roll_affix(&t,&mut it,true,true,false,false,0,0);
        let eligible=t.magic.iter().any(|row|row.level<=alvl(ilvl,0,0) && row.spawnable!=0 && row.frequency!=0);
        prop_assert_eq!(id!=0,eligible);
        if id!=0 {
            let row=affix(&t,id).unwrap();
            prop_assert!(row.level<=alvl(ilvl,0,0) && row.spawnable!=0 && row.frequency!=0, "id={id}, row={row:?}");
            prop_assert!(magic_fits(&t,&it,row));
        }
    }
}
