//! Level and parent assignment for aligned contents entries.
//!
//! Priority is explicit numbering, then contents-column indentation, then
//! relative line height, and finally the global consistency of neighbouring
//! entries. A level is never allowed to jump more than one step, and an
//! entry whose depth cannot be decided uniquely is deepened conservatively
//! (shallower) and marked for review rather than guessed.

use super::config::AutoBookmarkConfig;

#[derive(Debug, Clone)]
pub(crate) struct LevelDecision {
    pub(crate) level: u16,
    pub(crate) reason: String,
    pub(crate) ambiguous: bool,
}

/// Decides a level per contents entry, in contents order.
pub(crate) fn levels(
    entries: &[(Option<Vec<u32>>, u32, f64)],
    measured_geometry: bool,
    config: &AutoBookmarkConfig,
) -> Vec<LevelDecision> {
    let numbered = entries
        .iter()
        .filter(|(path, _, _)| path.as_ref().is_some_and(|path| !path.is_empty()))
        .count();
    let mut indents: Vec<u32> = entries.iter().map(|(_, indent, _)| *indent).collect();
    indents.sort_unstable();
    indents.dedup();
    // Line heights are compared in 10% buckets relative to the tallest
    // contents line: OCR line boxes are not font metrics, and a one-pixel
    // difference must never invent a hierarchy level.
    let tallest = entries
        .iter()
        .map(|(_, _, height)| *height)
        .fold(0.0_f64, f64::max);
    let bucket = |height: f64| -> i64 {
        if tallest <= 0.0 {
            0
        } else {
            ((height / tallest) * 10.0).round() as i64
        }
    };
    let mut heights: Vec<i64> = entries
        .iter()
        .map(|(_, _, height)| bucket(*height))
        .collect();
    heights.sort_unstable();
    heights.dedup();
    heights.reverse();

    let mut decisions: Vec<LevelDecision> = Vec::with_capacity(entries.len());
    for (path, indent, height) in entries {
        let decision = match path {
            Some(path) if !path.is_empty() && numbered * 2 >= entries.len() => LevelDecision {
                level: (path.len() as u16 - 1).min(config.max_auto_level),
                reason: "numbering_path".to_owned(),
                ambiguous: false,
            },
            _ if measured_geometry && indents.len() > 1 && indents.len() <= 4 => {
                let rank = indents
                    .iter()
                    .position(|value| value == indent)
                    .unwrap_or(0) as u16;
                LevelDecision {
                    level: rank.min(config.max_auto_level),
                    reason: "indentation_cluster".to_owned(),
                    ambiguous: false,
                }
            }
            _ if measured_geometry && heights.len() > 1 && heights.len() <= 3 => {
                let key = bucket(*height);
                let rank = heights.iter().position(|value| *value == key).unwrap_or(0) as u16;
                LevelDecision {
                    level: rank.min(config.max_auto_level),
                    reason: "relative_line_height".to_owned(),
                    ambiguous: true,
                }
            }
            _ => LevelDecision {
                level: 0,
                reason: "flat_default".to_owned(),
                ambiguous: false,
            },
        };
        decisions.push(decision);
    }
    // A level may never skip a step; when it would, the entry is pulled up to
    // the nearest legal depth and flagged.
    let mut previous = 0u16;
    for (position, decision) in decisions.iter_mut().enumerate() {
        if position == 0 {
            if decision.level > 0 {
                decision.level = 0;
                decision.ambiguous = true;
                decision.reason = format!("{}_root_normalized", decision.reason);
            }
        } else if decision.level > previous + 1 {
            decision.level = previous + 1;
            decision.ambiguous = true;
            decision.reason = format!("{}_level_clamped", decision.reason);
        }
        previous = decision.level;
    }
    decisions
}

/// Resolves each retained entry's parent as the nearest preceding retained
/// entry at a shallower level. A dropped parent promotes its children to the
/// nearest surviving ancestor instead of leaving a dangling reference.
pub(crate) fn parents(levels: &[u16], retained: &[bool], ids: &[String]) -> Vec<Option<String>> {
    let mut stack: Vec<(u16, String)> = Vec::new();
    let mut result = vec![None; levels.len()];
    for position in 0..levels.len() {
        if !retained[position] {
            continue;
        }
        while stack
            .last()
            .is_some_and(|(level, _)| *level >= levels[position])
        {
            stack.pop();
        }
        result[position] = stack.last().map(|(_, id)| id.clone());
        stack.push((levels[position], ids[position].clone()));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbering_paths_drive_levels_and_never_skip_a_step() {
        let config = AutoBookmarkConfig::default();
        let entries = vec![
            (Some(vec![1]), 0, 10.0),
            (Some(vec![1, 1]), 0, 10.0),
            (Some(vec![1, 1, 1, 1]), 0, 10.0),
        ];
        let decisions = levels(&entries, false, &config);
        assert_eq!(decisions[0].level, 0);
        assert_eq!(decisions[1].level, 1);
        assert_eq!(decisions[2].level, 2, "a level may not skip a step");
        assert!(decisions[2].ambiguous);
    }

    #[test]
    fn a_dropped_parent_promotes_its_children() {
        let ids: Vec<String> = ["a", "b", "c"].iter().map(|x| (*x).to_owned()).collect();
        let parents = parents(&[0, 1, 2], &[true, false, true], &ids);
        assert_eq!(parents[0], None);
        assert_eq!(parents[2].as_deref(), Some("a"));
    }
}
