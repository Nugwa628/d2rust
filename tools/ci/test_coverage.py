"""Numeric coverage merge and never-reached reports, without game files."""
import json
import tempfile
from pathlib import Path
import pytest
from tools.ci.coverage import merge, paths, main


def run(counts, rows=("0", "1", "2")):
    return {"coverage": {"format": "coverage 1", "counters": {"skills_cast": counts},
                         "tables": {"skills_cast": list(rows)}}}


def test_merge_preserves_exact_counts_and_lists_never_reached():
    result = merge([run({"0": 2, "1": 0}), run({"0": 3, "2": 1})])
    assert result["counters"]["skills_cast"] == {"0": 5, "1": 0, "2": 1}
    assert result["never_reached"] == {"skills_cast": ["1"]}
    assert result["summary"]["skills_cast"] == {"total": 3, "reached": 2, "never_reached": 1}


@pytest.mark.parametrize("count", [-1, True, 1.5, "1"])
def test_rejects_invalid_counters(count):
    with pytest.raises(ValueError):
        merge([run({"0": count})])


def test_missing_envelope_is_rejected():
    with pytest.raises(ValueError):
        merge([{"format": "repro 1"}])


def test_cli_directory_deduplicates_paths_and_avoids_aggregate():
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        report = root / "case" / "repro.json"
        report.parent.mkdir()
        report.write_text(json.dumps(run({"1": 4})), encoding="utf-8")
        (root / "sweep.json").write_text("{}", encoding="utf-8")
        assert paths([root, report]) == [report.resolve()]
        out = root / "merged.json"
        assert main([str(root), "--out", str(out)]) == 0
        assert json.loads(out.read_text())["never_reached"]["skills_cast"] == ["0", "2"]


def test_union_of_table_universes_and_no_hits():
    result = merge([run({}, ("1", "2")), run({}, ("2", "3"))])
    assert result["never_reached"]["skills_cast"] == ["1", "2", "3"]
