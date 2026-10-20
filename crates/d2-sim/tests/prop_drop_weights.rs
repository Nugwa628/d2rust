//! Compile synthetic treasure rows through the runtime builder, then exhaust
//! every possible roll and compare the selected frequencies with table weights.
use d2_data::{
    fixup::maps::EquivMatrix,
    tables::{Record, Treasureclassex},
};
use d2_sim::treasure::{select_entry, ItemData, TcSources, TreasureClasses};
use proptest::prelude::*;
fn item(code: [u8; 4], version: u16) -> ItemData {
    ItemData {
        code,
        ubercode: code,
        ultracode: code,
        version,
        level: 1,
        type_: 0,
        type2: 0,
        unique: 0,
        quest: 0,
        spawnable: 1,
    }
}
proptest! {
    #[test]
    fn runtime_weights_and_selection_match_tables(a in 1u32..40,b in 1u32..40,c in 1u32..40, expansion in any::<bool>()) {
        let mut row=Treasureclassex::decode(&vec![0;Treasureclassex::SIZE]);
        row.treasure_class[..9].copy_from_slice(b"synthetic");
        row.item1[..3].copy_from_slice(b"aaa");row.prob1=a;
        row.item2[..3].copy_from_slice(b"bbb");row.prob2=b;
        row.item3[..3].copy_from_slice(b"ccc");row.prob3=c;
        let items=[item(*b"aaa ",0),item(*b"bbb ",100),item(*b"ccc ",0)];
        let tcs=TreasureClasses::build(&TcSources {treasureclassex:&[row], itemtypes:&[], items:&items,equiv:&EquivMatrix::default(),uniqueitems:&[],setitems:&[]}).unwrap();
        let tc=&tcs.tcs[1];
        let weights=if expansion {[a,b,c]} else {[a,0,c]};
        prop_assert_eq!(tc.total(expansion),weights.iter().sum::<u32>() as i32);
        let mut counts=[0u32;3];
        for roll in 0..tc.total(expansion) {
            let i=select_entry(tc,roll,expansion).unwrap();
            counts[tc.entries[i].id as usize]+=1;
        }
        prop_assert_eq!(counts,weights);
    }
}
