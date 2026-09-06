"""Metric and ordering contract checks; synthetic boxes only, no Gold files."""
from run_dev_geometry_bakeoff import layout_order, metrics, summarize, xy_order


def box(x, y, w=80, h=10):
    return {'bbox': [x, y, x+w, y+h]}


def test_column_order_does_not_interleave_rows():
    lines = [box(120, 0), box(0, 20), box(0, 0), box(120, 20)]
    expected = [lines[2], lines[1], lines[0], lines[3]]
    assert xy_order(lines) == expected
    assert layout_order(lines, [{'coordinate': [0, 0, 90, 50]},
                                {'coordinate': [110, 0, 220, 50]}]) == expected


def test_order_is_accuracy_not_tau_and_missing_lines_keep_denominator():
    gold = [box(0, 0), box(0, 20), box(0, 40)]
    reverse = summarize([metrics(gold, list(reversed(gold)))])
    assert reverse['f1'] == 1 and reverse['order_accuracy'] == 0
    assert reverse['pair_coverage'] == 1
    sparse = summarize([metrics(gold, gold[:1])])
    assert sparse['order_accuracy'] is None
    assert sparse['pair_coverage'] == 0 and sparse['all_gold_coverage'] == 1/3


def test_extra_duplicate_is_penalized_and_failure_is_not_omitted():
    gold = [box(0, 0), box(0, 20)]
    duplicate = summarize([metrics(gold, gold + [gold[0]])])
    assert duplicate['matched'] == 2 and duplicate['precision'] == 2/3
    combined = summarize([metrics(gold, gold), metrics(gold, [])])
    assert combined['gold'] == 4 and combined['recall'] == .5


def test_layout_regions_cannot_drop_unassigned_lines():
    lines = [box(0, 0), box(0, 30), box(200, 60)]
    ordered = layout_order(lines, [{'coordinate': [0, 0, 100, 20]}])
    assert len(ordered) == len(lines)
    assert sorted(x['bbox'] for x in ordered) == sorted(x['bbox'] for x in lines)
