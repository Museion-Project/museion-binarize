//! Whole-book numbering grammar for the editable local TOC route.
//! Never reads assessment data. OCR label hypotheses remain explicit in reasons.
#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Decimal(Vec<u32>),
    Arabic(u32),
    Roman(u32),
    Upper(u32),
    Lower(u32),
    Keyword(u8, u32),
    Named(String),
    Unknown,
}
#[derive(Clone, Debug)]
struct Label {
    kind: Kind,
    raw: String,
    normalized: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Decision {
    pub parent: Option<usize>,
    pub level: u16,
    pub reason: String,
    pub suspect: bool,
    pub label: Option<String>,
    pub label_hypotheses: Vec<Option<String>>,
    pub parent_hypotheses: Vec<Option<usize>>,
}
fn roman(s: &str) -> Option<u32> {
    super::toc_parse::parse_roman(s)
}
fn decimal(s: &str) -> Option<Vec<u32>> {
    let s = s.trim_matches(|c: char| matches!(c, '.' | ')' | '('));
    if !s.contains('.') {
        return None;
    }
    s.split('.')
        .filter(|p| !p.is_empty())
        .map(|p| {
            p.chars()
                .map(|c| match c {
                    'l' | 'I' | 'i' => '1',
                    v => v,
                })
                .collect::<String>()
                .parse::<u32>()
                .ok()
        })
        .collect::<Option<Vec<_>>>()
        .filter(|p| p.len() > 1)
}
fn parse(text: &str) -> Label {
    let mut words = text.split_whitespace();
    let raw = words.next().unwrap_or("");
    let first = raw.trim_matches(|c: char| matches!(c, '.' | ')' | '('));
    let lower = first.to_lowercase();
    let named = [
        "preface",
        "introduction",
        "conclusion",
        "conclusions",
        "notes",
        "bibliography",
        "index",
        "indices",
    ];
    if named.contains(&lower.as_str()) {
        return Label {
            kind: Kind::Named(lower),
            raw: raw.into(),
            normalized: false,
        };
    }
    let keyword = match lower.as_str() {
        "part" => Some(0),
        "book" => Some(1),
        "chapter" => Some(2),
        "section" => Some(3),
        _ => None,
    };
    if let Some(rank) = keyword {
        let n = words
            .next()
            .unwrap_or("")
            .trim_matches(|c: char| !c.is_alphanumeric());
        let value = n.parse().ok().or_else(|| roman(n)).unwrap_or(0);
        return Label {
            kind: Kind::Keyword(rank, value),
            raw: format!("{raw} {n}"),
            normalized: value == 0,
        };
    }
    if let Some(path) = decimal(raw) {
        let normalized = raw.chars().any(|c| matches!(c, 'l' | 'I' | 'i')) || raw.contains("..");
        return Label {
            kind: Kind::Decimal(path),
            raw: raw.into(),
            normalized,
        };
    }
    let kind = if first == "l" && raw.ends_with('.') {
        Kind::Arabic(1)
    } else if let Ok(n) = first.parse::<u32>() {
        Kind::Arabic(n)
    } else if first.len() == 1 && first.chars().all(|c| ('A'..='H').contains(&c)) {
        Kind::Upper((first.as_bytes()[0] - b'A' + 1) as u32)
    } else if first.len() == 1
        && raw.ends_with(')')
        && first.chars().all(|c| ('a'..='z').contains(&c))
    {
        Kind::Lower((first.as_bytes()[0] - b'a' + 1) as u32)
    } else if first.chars().all(|c| "IVXLCDM".contains(c)) && !first.is_empty() {
        roman(first).map(Kind::Roman).unwrap_or(Kind::Unknown)
    } else {
        Kind::Unknown
    };
    Label {
        kind,
        raw: raw.into(),
        normalized: first == "l",
    }
}
fn canonical(k: &Kind) -> Option<String> {
    match k {
        Kind::Decimal(p) => Some(p.iter().map(u32::to_string).collect::<Vec<_>>().join(".")),
        Kind::Arabic(n) | Kind::Roman(n) => Some(n.to_string()),
        Kind::Upper(n) => char::from_u32(64 + n).map(|c| c.to_string()),
        Kind::Lower(n) => char::from_u32(96 + n).map(|c| c.to_string()),
        Kind::Keyword(r, n) => Some(format!(
            "{} {n}",
            ["PART", "BOOK", "CHAPTER", "SECTION"][*r as usize]
        )),
        _ => None,
    }
}
/// Inputs are title, normalized title x, has printed label. All decisions share
/// a book-wide label inventory; a broken prefix is never normalized silently.
pub(super) fn resolve(input: &[(String, f64, bool)]) -> Vec<Decision> {
    let mut labels: Vec<_> = input.iter().map(|(s, _, _)| parse(s)).collect();
    // Recover only malformed prefix hypotheses bracketed by two explicit decimal
    // siblings. Text itself and raw label remain unchanged.
    for i in 0..labels.len() {
        let raw = &labels[i].raw;
        let malformed = raw.chars().any(|c| c.is_ascii_digit())
            && (raw.contains('-')
                || matches!(labels[i].kind, Kind::Arabic(_))
                    && input[i]
                        .0
                        .split_whitespace()
                        .nth(1)
                        .is_some_and(|s| s.chars().all(|c| c.is_ascii_digit())));
        let alphabetic_neighbor = raw.contains('4')
            && (raw.starts_with('.') || raw.starts_with('-'))
            && (i + 1..labels.len())
                .take(4)
                .any(|j| labels[j].kind == Kind::Upper(2));
        if malformed && !alphabetic_neighbor {
            let prev = (0..i).rev().find_map(|j| {
                if let Kind::Decimal(p) = &labels[j].kind {
                    Some(p.clone())
                } else {
                    None
                }
            });
            let next = (i + 1..labels.len()).find_map(|j| {
                if let Kind::Decimal(p) = &labels[j].kind {
                    Some(p.clone())
                } else {
                    None
                }
            });
            if let (Some(a), Some(b)) = (prev, next) {
                let candidate = if a.len() == b.len()
                    && a[..a.len() - 1] == b[..b.len() - 1]
                    && b[b.len() - 1] == a[a.len() - 1] + 2
                {
                    let mut p = a;
                    p.last_mut().map(|n| *n += 1);
                    Some(p)
                } else if b.len() == a.len() + 1 && b[..a.len()] == a && b[b.len() - 1] == 2 {
                    let mut p = a;
                    p.push(1);
                    Some(p)
                } else {
                    None
                };
                if let Some(p) = candidate {
                    labels[i].kind = Kind::Decimal(p);
                    labels[i].normalized = true
                }
            }
        }
    }
    // A/4 and a/ti) confusion needs an actual following alphabetic sibling;
    // it is a hypothesis, not character correction applied to the title.
    for i in 0..labels.len() {
        if labels[i].raw.contains('4')
            && !matches!(labels[i].kind, Kind::Decimal(_))
            && (labels[i].raw.starts_with('.') || labels[i].raw.starts_with('-'))
            && (i + 1..labels.len())
                .take(4)
                .any(|j| labels[j].kind == Kind::Upper(2))
        {
            labels[i].kind = Kind::Upper(1);
            labels[i].normalized = true
        }
        if matches!(labels[i].kind, Kind::Unknown)
            && labels[i].raw.ends_with(')')
            && labels[i].raw.len() <= 4
            && labels.get(i + 1).is_some_and(|l| l.kind == Kind::Lower(2))
        {
            labels[i].kind = Kind::Lower(1);
            labels[i].normalized = true
        }
    }
    // Keep the literal (unparsed) reading alongside the interpreted label.
    // A confusable decimal needs another explicit prefix/child/sibling in this
    // book; isolated shape normalization cannot establish a section number.
    let proposed: Vec<_> = labels.iter().map(|l| canonical(&l.kind)).collect();
    let inventory = labels.clone();
    for (i, l) in labels.iter_mut().enumerate() {
        if l.normalized {
            if let Kind::Decimal(path) = &l.kind {
                let supported = inventory.iter().enumerate().any(|(j, other)| {
                    if i == j || other.normalized {
                        return false;
                    }
                    match &other.kind {
                        Kind::Decimal(q) => {
                            q != path
                                && (path.starts_with(q)
                                    || q.starts_with(path)
                                    || q.len() == path.len()
                                        && q[..q.len() - 1] == path[..path.len() - 1])
                        }
                        Kind::Arabic(n) => path[0] == *n,
                        _ => false,
                    }
                });
                if !supported {
                    l.kind = Kind::Unknown;
                }
            }
        }
    }
    let numbered_roots = labels.iter().enumerate().any(|(i, l)| {
        if let Kind::Arabic(n) = l.kind {
            labels.iter().enumerate().any(|(j, k)| {
                matches!(&k.kind,Kind::Decimal(p) if p[0]==n)
                    && (!input[i].2 || input[i].1 <= input[j].1 + 0.005)
            })
        } else {
            false
        }
    });
    let min_depth = if numbered_roots {
        1
    } else {
        labels
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                if let Kind::Decimal(p) = &l.kind {
                    input[i].2.then_some(p.len())
                } else {
                    None
                }
            })
            .min()
            .unwrap_or(1)
    };
    let mut out: Vec<Decision> = vec![];
    let mut last_decimal = None;
    let mut last_upper = None;
    let mut last_arabic = None;
    let mut last_roman = None;
    let mut last_index = None;
    let mut keyword_stack: Vec<(u8, usize)> = vec![];
    for i in 0..labels.len() {
        let l = &labels[i];
        let mut suspect = l.normalized;
        let mut reason = "book_sequence_grammar".to_string();
        let mut parent = match &l.kind {
            Kind::Named(name) => {
                // Names are weak cues: use the active ancestry and measured
                // indentation, retaining the competing root/scope reading.
                let mut ancestor = i.checked_sub(1);
                let mut scoped = None;
                while let Some(j) = ancestor {
                    if input[i].1 > input[j].1 + 0.01 {
                        scoped = Some(j);
                        break;
                    }
                    ancestor = out[j].parent;
                }
                suspect |= i > 0;
                reason = if scoped.is_some() {
                    "named_scope_from_layout"
                } else {
                    "named_root_from_layout"
                }
                .into();
                last_decimal = None;
                last_upper = None;
                last_arabic = None;
                last_roman = None;
                if let Some(j) = scoped {
                    keyword_stack.retain(|(_, k)| *k <= j);
                } else {
                    keyword_stack.clear();
                }
                last_index = if name == "index" || name == "indices" {
                    Some(i)
                } else {
                    None
                };
                scoped
            }
            Kind::Keyword(rank, _) => {
                while keyword_stack.last().is_some_and(|(r, _)| r >= rank) {
                    keyword_stack.pop();
                }
                let p = keyword_stack.last().map(|(_, j)| *j);
                keyword_stack.push((*rank, i));
                last_decimal = None;
                last_upper = None;
                last_index = None;
                p
            }
            Kind::Decimal(path) => {
                let p = {
                    (0..i).rev().find(|j|matches!(&labels[*j].kind,Kind::Decimal(q) if q.len()+1==path.len() && path.starts_with(q)) || numbered_roots && path.len()==2 && out[*j].parent.is_none() && matches!(labels[*j].kind,Kind::Arabic(n) if n==path[0]))
                };
                if path.len() > min_depth && p.is_none() {
                    suspect = true;
                    reason = "decimal_parent_missing".into()
                }
                last_decimal = Some(i);
                last_upper = None;
                last_index = None;
                p
            }
            Kind::Upper(_) => {
                let p = last_decimal
                    .or(last_arabic)
                    .or(last_roman)
                    .or(keyword_stack.last().map(|(_, j)| *j));
                last_upper = Some(i);
                p
            }
            Kind::Lower(_) => last_upper.or(last_decimal),
            Kind::Arabic(n) => {
                let numeric_root = numbered_roots
                    && labels.iter().enumerate().any(|(j, l)| {
                        matches!(&l.kind,Kind::Decimal(p) if p[0]==*n)
                            && (!input[i].2 || input[i].1 <= input[j].1 + 0.005)
                    });
                let sequence_root = numbered_roots
                    && (0..i)
                        .rev()
                        .find(|j| {
                            matches!(labels[*j].kind, Kind::Arabic(_)) && out[*j].parent.is_none()
                        })
                        .is_some_and(|j| {
                            matches!(labels[j].kind,Kind::Arabic(prev) if *n==prev+1)
                                && (last_upper.is_none() || (input[i].1 - input[j].1).abs() < 0.01)
                        });
                let numeric_root = numeric_root || sequence_root;
                let p = if numeric_root {
                    last_decimal = None;
                    last_upper = None;
                    None
                } else if let Some(j) = last_index {
                    Some(j)
                } else if last_decimal.is_some() {
                    last_upper.or(last_decimal)
                } else {
                    keyword_stack.last().map(|(_, j)| *j).or(last_roman)
                };
                // Numeric siblings retain the same scope; repeated/nonsequential numbers
                // raise a review reason rather than invent a new ancestor.
                if let Some(j) = last_arabic {
                    if let Kind::Arabic(prev) = labels[j].kind {
                        if *n != 1 && *n != prev + 1 && out[j].parent == p {
                            suspect = true;
                            reason = "numeric_sequence_gap".into()
                        }
                    }
                }
                last_arabic = Some(i);
                p
            }
            Kind::Roman(_) => {
                last_roman = Some(i);
                last_arabic = None;
                keyword_stack.last().map(|(_, j)| *j)
            }
            Kind::Unknown => {
                reason = "unnumbered_root".into();
                if let Some(j) = last_decimal {
                    if input[i].1 > input[j].1 + 0.004 {
                        suspect = true;
                        reason = "unlabeled_indented_entry".into();
                        Some(j)
                    } else {
                        suspect = true;
                        None
                    }
                } else {
                    None
                }
            }
        };
        let mut parent_hypotheses = vec![parent];
        // A closed branch cannot be reopened in a source-ordered outline.
        // Keep that reading as a hypothesis, but expose a reviewable root
        // instead of producing an invalid tree or reordering source evidence.
        if let Some(proposed_parent) = parent {
            let mut ancestor = i.checked_sub(1);
            while ancestor.is_some() && ancestor != Some(proposed_parent) {
                ancestor = out[ancestor.unwrap()].parent;
            }
            if ancestor.is_none() {
                parent = None;
                parent_hypotheses.push(None);
                suspect = true;
                reason = "closed_parent_requires_review".into();
            }
        }
        if matches!(l.kind, Kind::Named(_)) {
            let scope = i.checked_sub(1).and_then(|j| out[j].parent.or(Some(j)));
            for alternative in [None, scope] {
                if !parent_hypotheses.contains(&alternative) {
                    parent_hypotheses.push(alternative);
                }
            }
        }
        let label_hypotheses = if l.normalized {
            vec![proposed[i].clone(), None]
        } else {
            vec![canonical(&l.kind)]
        };
        let level = parent.map(|j| out[j].level + 1).unwrap_or(0);
        out.push(Decision {
            parent,
            level,
            reason: if l.normalized {
                format!("{reason}:pattern_normalization")
            } else {
                reason
            },
            suspect,
            label: canonical(&l.kind),
            label_hypotheses,
            parent_hypotheses,
        });
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_branch_is_retained_as_hypothesis_without_reordering() {
        let d = resolve(&[
            ("1.1 Section".into(), 0.1, true),
            ("Unrelated root".into(), 0.1, true),
            ("A. Possible child".into(), 0.15, true),
        ]);
        assert_eq!(d[2].parent, None);
        assert!(d[2].suspect);
        assert!(d[2].parent_hypotheses.contains(&Some(0)));
        assert_eq!(d[2].reason, "closed_parent_requires_review");
    }
    fn run(s: &[&str]) -> Vec<Decision> {
        resolve(
            &s.iter()
                .map(|s| (s.to_string(), 0.1, true))
                .collect::<Vec<_>>(),
        )
    }
    #[test]
    fn decimal_sequence() {
        let d = run(&["1 One", "1.1 First", "1.2 Second", "2 Two", "2.1 Third"]);
        assert_eq!(
            d.iter().map(|d| d.parent).collect::<Vec<_>>(),
            vec![None, Some(0), Some(0), None, Some(3)]
        );
    }
    #[test]
    fn numeric_sibling_does_not_need_a_following_child_to_be_root() {
        let d = run(&["1 First", "1.1 Child", "1.2 Child", "2 Next"]);
        assert_eq!(d[3].parent, None);
    }
    #[test]
    fn confusable_decimal_survives() {
        let d = run(&["1.1 Title", "l.l.I Child", "1.1.2 Other"]);
        assert_eq!(d[1].parent, Some(0));
        assert!(d[1].suspect);
        assert_eq!(d[1].label.as_deref(), Some("1.1.1"));
    }
    #[test]
    fn front_back_matter_roots() {
        let d = run(&[
            "Preface",
            "1 First",
            "2 Second",
            "Conclusion",
            "Notes",
            "Bibliography",
            "Index",
            "l. Names",
            "2. Subjects",
        ]);
        assert!(d[..7].iter().all(|d| d.parent.is_none()));
        assert_eq!(d[7].parent, Some(6));
        assert_eq!(d[8].parent, Some(6));
    }
    #[test]
    fn explicit_keywords() {
        let d = run(&[
            "PART I Title",
            "BOOK I Title",
            "CHAPTER 1 Title",
            "SECTION A Title",
            "CHAPTER 2 Title",
            "PART II Title",
        ]);
        assert_eq!(
            d.iter().map(|d| d.parent).collect::<Vec<_>>(),
            vec![None, Some(0), Some(1), Some(2), Some(1), None]
        );
    }
    #[test]
    fn roman_sequence_and_letters() {
        let d = run(&[
            "I First",
            "1 First chapter",
            "A. Detail",
            "B. Detail",
            "2 Next chapter",
            "II Second",
            "1 First chapter",
        ]);
        assert_eq!(d[3].parent, Some(1));
        assert_eq!(d[4].parent, Some(0));
        assert_eq!(d[6].parent, Some(5));
    }
    #[test]
    fn isolated_confusable_number_is_not_promoted() {
        let d = run(&["l.l.I A title"]);
        assert_eq!(d[0].label, None);
        assert_eq!(d[0].label_hypotheses, vec![Some("1.1.1".into()), None]);
        assert!(d[0].suspect);
    }
    #[test]
    fn named_sections_follow_active_layout_scope() {
        let d = resolve(&[
            ("CHAPTER 1 First".into(), 0.1, true),
            ("SECTION 1 Detail".into(), 0.16, true),
            ("Notes".into(), 0.16, true),
            ("CHAPTER 2 Second".into(), 0.1, true),
            ("Conclusion".into(), 0.1, true),
        ]);
        assert_eq!(d[2].parent, Some(0));
        assert!(d[2].parent_hypotheses.contains(&None));
        assert!(d[2].suspect);
        assert_eq!(d[3].parent, None);
        assert_eq!(d[4].parent, None);
    }
    #[test]
    fn centered_container_does_not_turn_nested_numbers_into_chapters() {
        let d = resolve(&[
            ("1. Container".into(), 0.6, false),
            ("1.1 Topic".into(), 0.3, true),
            ("1.1.1 Detail".into(), 0.3, true),
            ("A. Group".into(), 0.35, true),
            ("1. First".into(), 0.4, true),
            ("2. Second".into(), 0.4, true),
            ("B. Next group".into(), 0.35, true),
            ("1.2 Next topic".into(), 0.3, true),
        ]);
        assert_eq!(d[5].parent, Some(3));
        assert_eq!(d[6].parent, Some(2));
        assert_eq!(d[7].parent, Some(0));
    }
}
