import run_geometry_bakeoff as bakeoff


def test_matching_and_reading_order_are_scored_separately():
    gold = [
        {"bbox": [0, 0, 100, 10]},
        {"bbox": [0, 20, 100, 30]},
    ]
    reversed_candidate = [
        {"bbox": [0, 20, 100, 30]},
        {"bbox": [0, 0, 100, 10]},
    ]
    metrics = bakeoff.evaluate_page(gold, reversed_candidate)
    assert metrics["f1"] == 1.0
    assert metrics["mean_iou"] == 1.0
    assert metrics["reading_order_tau"] == -1.0


def test_split_or_merged_lines_do_not_pass_one_to_one_contract():
    gold = [
        {"bbox": [0, 0, 100, 10]},
        {"bbox": [0, 12, 100, 22]},
    ]
    merged = [{"bbox": [0, 0, 100, 22]}]
    metrics = bakeoff.evaluate_page(gold, merged)
    assert metrics["matched_lines"] == 1
    assert metrics["recall"] == 0.5
