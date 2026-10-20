//! Additive stat lists have no clamp or op; bounded positive additions must
//! produce the arithmetic sum and cannot lower the authoritative total.
use d2_data::{
    bin::BinTable,
    tables::{Itemstatcost, Record},
};
use d2_sim::stats::{
    lists::{NoHost, StatLists},
    StatData,
};
use proptest::prelude::*;
use std::sync::Arc;
proptest! {
    #[test]
    fn stat_totals_equal_sum_and_never_decrease(base in 0i32..=1_000_000, additions in prop::collection::vec(0i32..=10_000, 0..100)) {
        let mut records=vec![0;22*Itemstatcost::SIZE];
        // Fixed-up dependency lists end with a stat outside this table.
        for record in records.as_chunks_mut::<{ Itemstatcost::SIZE }>().0 {
            record[0x5e..0x60].copy_from_slice(&u16::MAX.to_le_bytes());
        }
        let table=BinTable{name:"itemstatcost".into(),source:"synthetic".into(),count:22,record_size:Itemstatcost::SIZE,records};
        let data=StatData{stats:d2_sim::stats::StatTable::from_fixed(&table).unwrap(),..StatData::default()};
        let mut lists=StatLists::new(Arc::new(data));
        let list=lists.alloc(0,0,0,1);
        lists.set(&mut NoHost,list,21,base,0,None);
        let mut expected=base;
        for value in additions {
            let previous=lists.total(list,21,0);
            lists.add(&mut NoHost,list,21,value,0);
            expected+=value;
            let actual=lists.total(list,21,0);
            prop_assert_eq!(actual,expected);
            prop_assert!(actual>=previous && actual<=2_000_000, "total={actual}, previous={previous}");
        }
    }
}
