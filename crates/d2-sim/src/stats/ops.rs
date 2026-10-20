//! The op table (`sim/stat-ops.tsv`) as data: per op, the guard, the
//! prev rule, the operand, the contribution and the recompute block.
//! The test `table_matches_tsv` checks [`OP_ROWS`] against the TSV;
//! evaluation is in [`super::lists`].

/// Guard: skip the entry unless true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guard {
    Owner,
    OwnerItem,
    OwnerPlayer,
    OwnerPmPrev,
    OwnerAct,
    ListtypePm,
    UnitPm,
    Never,
}

/// prev rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prev {
    OwnerItemBase,
    Keep,
}

/// Operand x (ops 2–5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operand {
    None,
    OpbaseListTotal,
    OpbaseUnitTotal,
}

/// Contribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contribution {
    None,
    MuldivPrevR,
    ShiftRX,
    MuldivPrevShiftRX,
    BytimeR,
    MuldivAccBytimeR,
    CharstatManaBonus,
    CharstatVitBonus,
}

/// The recompute block of the op evaluation, step 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecomputeBlock {
    None,
    Always,
    ListtypePm,
    ListtypeItem,
    ListtypePmAndEntrybaseListTotalPos,
    UnitPmAndEntrybaseUnitTotalPos,
}

/// One row of `stat-ops.tsv`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpRow {
    pub op: u8,
    pub guard: Guard,
    pub prev: Prev,
    pub operand: Operand,
    pub contribution: Contribution,
    pub recompute_block: RecomputeBlock,
}

const fn row(
    op: u8,
    guard: Guard,
    prev: Prev,
    operand: Operand,
    contribution: Contribution,
    recompute_block: RecomputeBlock,
) -> OpRow {
    OpRow {
        op,
        guard,
        prev,
        operand,
        contribution,
        recompute_block,
    }
}

use Contribution as C;
use Guard as G;
use Operand as O;
use RecomputeBlock as B;

/// Ops 1–13 (`stat-ops.tsv`, row k = op k + 1).
pub const OP_ROWS: [OpRow; 13] = [
    row(
        1,
        G::Owner,
        Prev::OwnerItemBase,
        O::None,
        C::MuldivPrevR,
        B::Always,
    ),
    row(
        2,
        G::ListtypePm,
        Prev::Keep,
        O::OpbaseListTotal,
        C::ShiftRX,
        B::ListtypePmAndEntrybaseListTotalPos,
    ),
    row(
        3,
        G::ListtypePm,
        Prev::Keep,
        O::OpbaseListTotal,
        C::MuldivPrevShiftRX,
        B::ListtypePmAndEntrybaseListTotalPos,
    ),
    row(
        4,
        G::UnitPm,
        Prev::Keep,
        O::OpbaseUnitTotal,
        C::ShiftRX,
        B::UnitPmAndEntrybaseUnitTotalPos,
    ),
    row(
        5,
        G::UnitPm,
        Prev::Keep,
        O::OpbaseUnitTotal,
        C::MuldivPrevShiftRX,
        B::UnitPmAndEntrybaseUnitTotalPos,
    ),
    row(
        6,
        G::OwnerAct,
        Prev::Keep,
        O::None,
        C::BytimeR,
        B::ListtypePm,
    ),
    row(
        7,
        G::OwnerAct,
        Prev::Keep,
        O::None,
        C::MuldivAccBytimeR,
        B::ListtypePm,
    ),
    row(
        8,
        G::OwnerPlayer,
        Prev::Keep,
        O::None,
        C::CharstatManaBonus,
        B::None,
    ),
    row(
        9,
        G::OwnerPlayer,
        Prev::Keep,
        O::None,
        C::CharstatVitBonus,
        B::None,
    ),
    row(10, G::Never, Prev::Keep, O::None, C::None, B::None),
    row(
        11,
        G::OwnerPmPrev,
        Prev::Keep,
        O::None,
        C::MuldivPrevR,
        B::None,
    ),
    row(12, G::Never, Prev::Keep, O::None, C::None, B::Always),
    row(
        13,
        G::OwnerItem,
        Prev::OwnerItemBase,
        O::None,
        C::MuldivPrevR,
        B::ListtypeItem,
    ),
];

/// The row of op `op` (1–13), else none (ops > 13 are skipped, op 0 ends
/// a table).
pub fn op_row(op: u8) -> Option<&'static OpRow> {
    OP_ROWS.get(usize::from(op).checked_sub(1)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TSV: &str = include_str!("../../../../tables/sim/stat-ops.tsv");

    fn guard(s: &str) -> Guard {
        match s {
            "owner" => G::Owner,
            "owner_item" => G::OwnerItem,
            "owner_player" => G::OwnerPlayer,
            "owner_pm_prev" => G::OwnerPmPrev,
            "owner_act" => G::OwnerAct,
            "listtype_pm" => G::ListtypePm,
            "unit_pm" => G::UnitPm,
            "never" => G::Never,
            _ => panic!("unknown guard {s}"),
        }
    }

    fn prev(s: &str) -> Prev {
        match s {
            "owner_item_base" => Prev::OwnerItemBase,
            "keep" => Prev::Keep,
            _ => panic!("unknown prev {s}"),
        }
    }

    fn operand(s: &str) -> Operand {
        match s {
            "none" => O::None,
            "opbase_list_total" => O::OpbaseListTotal,
            "opbase_unit_total" => O::OpbaseUnitTotal,
            _ => panic!("unknown operand {s}"),
        }
    }

    fn contribution(s: &str) -> Contribution {
        match s {
            "none" => C::None,
            "muldiv_prev_r" => C::MuldivPrevR,
            "shift_r_x" => C::ShiftRX,
            "muldiv_prev_shift_r_x" => C::MuldivPrevShiftRX,
            "bytime_r" => C::BytimeR,
            "muldiv_acc_bytime_r" => C::MuldivAccBytimeR,
            "charstat_mana_bonus" => C::CharstatManaBonus,
            "charstat_vit_bonus" => C::CharstatVitBonus,
            _ => panic!("unknown contribution {s}"),
        }
    }

    fn block(s: &str) -> RecomputeBlock {
        match s {
            "none" => B::None,
            "always" => B::Always,
            "listtype_pm" => B::ListtypePm,
            "listtype_item" => B::ListtypeItem,
            "listtype_pm_and_entrybase_list_total_pos" => B::ListtypePmAndEntrybaseListTotalPos,
            "unit_pm_and_entrybase_unit_total_pos" => B::UnitPmAndEntrybaseUnitTotalPos,
            _ => panic!("unknown recompute_block {s}"),
        }
    }

    fn parse(tsv: &str) -> Vec<OpRow> {
        let mut lines = tsv.lines();
        assert_eq!(
            lines.next(),
            Some("op\tguard\tprev\toperand\tcontribution\trecompute_block\tnote")
        );
        lines
            .filter(|l| !l.is_empty())
            .map(|l| {
                let c: Vec<&str> = l.split('\t').collect();
                assert_eq!(c.len(), 7, "row {l}");
                row(
                    c[0].parse().expect("op"),
                    guard(c[1]),
                    prev(c[2]),
                    operand(c[3]),
                    contribution(c[4]),
                    block(c[5]),
                )
            })
            .collect()
    }

    #[test]
    fn table_matches_tsv() {
        assert_eq!(parse(TSV), OP_ROWS);
    }

    /// A changed TSV cell is reported.
    #[test]
    fn tsv_check_catches_perturbation() {
        let changed = TSV.replacen("4\tunit_pm\tkeep", "4\tlisttype_pm\tkeep", 1);
        assert_ne!(changed, TSV);
        let rows = parse(&changed);
        let diff: Vec<u8> = rows
            .iter()
            .zip(OP_ROWS.iter())
            .filter(|(a, b)| a != b)
            .map(|(a, _)| a.op)
            .collect();
        assert_eq!(diff, [4]);
    }

    #[test]
    fn rows_are_ops_in_order() {
        for (k, r) in OP_ROWS.iter().enumerate() {
            assert_eq!(usize::from(r.op), k + 1);
            assert_eq!(op_row(r.op), Some(r));
        }
        assert_eq!(op_row(0), None);
        assert_eq!(op_row(14), None);
    }
}
