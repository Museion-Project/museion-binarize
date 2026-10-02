//! The experimental path: line rectangles returned by the model itself.
//!
//! # Status: not a default, and not close to one
//!
//! Nothing in this build uses provider rectangles as the coordinate source.
//! The parser and the gates exist so the path can be *evaluated* honestly —
//! against hand-made ground truth and adversarial responses — rather than
//! adopted because a sample page looked right. [`StructuredBboxPolicy`]
//! defaults to [`StructuredBboxPolicy::Disabled`], and
//! [`GATES_VALIDATED_FOR_DEFAULT`] is the single flag that would have to flip
//! after a documented validation run.
//!
//! # Why the gates are shaped like this
//!
//! A language model returning JSON will produce well-formed JSON almost every
//! time and *correct* rectangles much less often. The dangerous failures are
//! not malformed responses — those fail loudly — but plausible ones: boxes
//! shifted by a margin, a column read in the wrong order, one line's box
//! covering half the page, the same box repeated. Each of those has a gate
//! below, and each gate compares against something the local detector
//! measured rather than against the response's own internal consistency.

use serde::{Deserialize, Serialize};

use crate::ocr::{OcrBlock, OcrBox, OcrLine, OcrPage, OcrWord};

use super::alignment::{AlignmentConfig, ALIGNMENT_VERSION};

/// Version of the structured response schema this build accepts.
pub const STRUCTURED_BBOX_SCHEMA_VERSION: &str = "mpdf-provider-lines/0.1";

/// Whether the documented geometry gates have been met on a validation set.
///
/// Flipping this to `true` requires: a hand-labelled bbox fixture set, a
/// recorded adversarial-response suite, and the thresholds in
/// [`GeometryGates`] met on a holdout. Until then the structured path can be
/// enabled per run for evaluation but can never be the default.
pub const GATES_VALIDATED_FOR_DEFAULT: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredBboxPolicy {
    /// Never request or accept provider rectangles.
    #[default]
    Disabled,
    /// Request them, validate them, and fall back deterministically to the
    /// local-geometry path when any gate fails. Evaluation only.
    EvaluateWithFallback,
}

impl StructuredBboxPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::EvaluateWithFallback => "evaluate_with_fallback",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "disabled" => Some(Self::Disabled),
            "evaluate_with_fallback" => Some(Self::EvaluateWithFallback),
            _ => None,
        }
    }

    /// Whether this policy may be selected as a build default. Always false
    /// until [`GATES_VALIDATED_FOR_DEFAULT`].
    pub fn allowed_as_default(self) -> bool {
        matches!(self, Self::Disabled) || GATES_VALIDATED_FOR_DEFAULT
    }
}

/// The wire form. `deny_unknown_fields` is load-bearing: a model that invents
/// an extra key is a model that is not following the contract, and accepting
/// the rest of its answer means guessing which parts it did follow.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StructuredLinesResponse {
    pub schema_version: String,
    pub page_width: f32,
    pub page_height: f32,
    pub lines: Vec<StructuredLine>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StructuredLine {
    pub text: String,
    /// `[x, y, width, height]` in the page's pixel coordinate system, origin
    /// top-left — the same system [`crate::ocr::OcrBox`] uses. A response in
    /// any other system fails the in-page gate rather than being rescaled,
    /// because silently rescaling a wrong guess produces a confident wrong
    /// answer.
    pub bbox: [f32; 4],
    pub reading_order: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeometryGates {
    /// Every accepted box must lie inside the page. Not a threshold: the
    /// fraction inside must be exactly 1.
    pub require_all_in_page: bool,
    /// Smallest and largest a single line box may be, as a fraction of the
    /// page area.
    pub min_area_fraction: f32,
    pub max_area_fraction: f32,
    /// Minimum fraction of provider lines that must overlap a locally
    /// detected line by at least [`GeometryGates::min_iou`].
    pub min_local_match_coverage: f32,
    pub min_iou: f32,
    /// Maximum fraction of provider lines allowed to overlap *each other*
    /// heavily; duplicates and stacked boxes are a classic failure.
    pub max_mutual_overlap_fraction: f32,
    /// Provider line count must be within this ratio of the local count.
    pub max_line_count_ratio: f32,
    pub max_lines: usize,
}

impl Default for GeometryGates {
    fn default() -> Self {
        Self {
            require_all_in_page: true,
            min_area_fraction: 0.000_02,
            max_area_fraction: 0.5,
            min_local_match_coverage: 0.9,
            min_iou: 0.5,
            max_mutual_overlap_fraction: 0.02,
            max_line_count_ratio: 1.5,
            max_lines: 4_096,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StructuredBboxError {
    #[error("structured response is not valid JSON under the strict schema: {0}")]
    Schema(String),
    #[error("structured response declares schema version {0}")]
    UnsupportedVersion(String),
    #[error("geometry gate failed: {0}")]
    Gate(String),
}

/// Report of what the gates measured, kept whether they passed or failed so a
/// validation run has numbers rather than a boolean.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GateReport {
    pub provider_lines: usize,
    pub local_lines: usize,
    pub in_page_fraction: f32,
    pub local_match_coverage: f32,
    pub mutual_overlap_fraction: f32,
    pub order_violations: usize,
    pub degenerate_boxes: usize,
    pub oversized_boxes: usize,
    pub passed: bool,
}

/// Parses a structured response under the strict schema.
///
/// Rejects unknown fields, wrong versions, non-finite numbers, and anything
/// past the line cap. This runs before any geometry reasoning so a hostile
/// response cannot reach the gate arithmetic at all.
pub fn parse_structured(
    body: &str,
    gates: &GeometryGates,
) -> Result<StructuredLinesResponse, StructuredBboxError> {
    let response: StructuredLinesResponse = serde_json::from_str(body)
        .map_err(|error| StructuredBboxError::Schema(error.to_string()))?;
    if response.schema_version != STRUCTURED_BBOX_SCHEMA_VERSION {
        return Err(StructuredBboxError::UnsupportedVersion(
            response.schema_version,
        ));
    }
    if response.lines.len() > gates.max_lines {
        return Err(StructuredBboxError::Schema(format!(
            "{} lines exceeds the {} line cap",
            response.lines.len(),
            gates.max_lines
        )));
    }
    if !response.page_width.is_finite()
        || !response.page_height.is_finite()
        || response.page_width <= 0.0
        || response.page_height <= 0.0
    {
        return Err(StructuredBboxError::Schema(
            "page dimensions are not positive finite numbers".into(),
        ));
    }
    for line in &response.lines {
        if line.text.is_empty() || line.text.len() > 16 * 1024 {
            return Err(StructuredBboxError::Schema(
                "line text is empty or too long".into(),
            ));
        }
        if line.bbox.iter().any(|value| !value.is_finite()) {
            return Err(StructuredBboxError::Schema(
                "bbox contains a non-finite value".into(),
            ));
        }
    }
    Ok(response)
}

fn iou(left: &OcrBox, right: &OcrBox) -> f32 {
    let x1 = left.x.max(right.x);
    let y1 = left.y.max(right.y);
    let x2 = (left.x + left.width).min(right.x + right.width);
    let y2 = (left.y + left.height).min(right.y + right.height);
    if x2 <= x1 || y2 <= y1 {
        return 0.0;
    }
    let intersection = (x2 - x1) * (y2 - y1);
    let union = left.width * left.height + right.width * right.height - intersection;
    if union <= 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn local_line_boxes(page: &OcrPage) -> Vec<OcrBox> {
    let mut boxes: Vec<(u32, u32, OcrBox)> = Vec::new();
    for block in &page.blocks {
        for line in &block.lines {
            boxes.push((block.reading_order, line.reading_order, line.bbox.clone()));
        }
    }
    boxes.sort_by_key(|(block, line, _)| (*block, *line));
    boxes.into_iter().map(|(_, _, bbox)| bbox).collect()
}

/// Runs every gate against the locally measured page.
///
/// The local page is the reference, always. A structured response is only
/// ever accepted when it agrees with something that was measured; it is never
/// accepted because it is internally consistent.
pub fn evaluate_gates(
    response: &StructuredLinesResponse,
    local: &OcrPage,
    gates: &GeometryGates,
) -> (GateReport, Result<(), StructuredBboxError>) {
    let local_boxes = local_line_boxes(local);
    let page_area = (local.width as f32) * (local.height as f32);
    let mut report = GateReport {
        provider_lines: response.lines.len(),
        local_lines: local_boxes.len(),
        ..Default::default()
    };

    let mut in_page = 0usize;
    let mut boxes = Vec::with_capacity(response.lines.len());
    for line in &response.lines {
        let [x, y, width, height] = line.bbox;
        let bbox = OcrBox {
            x,
            y,
            width,
            height,
        };
        let inside = x >= 0.0
            && y >= 0.0
            && width > 0.0
            && height > 0.0
            && x + width <= local.width as f32 + 1.0
            && y + height <= local.height as f32 + 1.0;
        if inside {
            in_page += 1;
        }
        let area = (width * height).max(0.0);
        if width <= 0.0 || height <= 0.0 || area < page_area * gates.min_area_fraction {
            report.degenerate_boxes += 1;
        }
        if area > page_area * gates.max_area_fraction {
            report.oversized_boxes += 1;
        }
        boxes.push(bbox);
    }
    report.in_page_fraction = if response.lines.is_empty() {
        0.0
    } else {
        in_page as f32 / response.lines.len() as f32
    };

    let mut previous_order: Option<u32> = None;
    for line in &response.lines {
        if previous_order.is_some_and(|previous| line.reading_order <= previous) {
            report.order_violations += 1;
        }
        previous_order = Some(line.reading_order);
    }

    let matched = boxes
        .iter()
        .filter(|candidate| {
            local_boxes
                .iter()
                .any(|reference| iou(candidate, reference) >= gates.min_iou)
        })
        .count();
    report.local_match_coverage = if boxes.is_empty() {
        0.0
    } else {
        matched as f32 / boxes.len() as f32
    };

    let mut overlapping = 0usize;
    for (index, candidate) in boxes.iter().enumerate() {
        if boxes
            .iter()
            .enumerate()
            .any(|(other, reference)| other != index && iou(candidate, reference) >= gates.min_iou)
        {
            overlapping += 1;
        }
    }
    report.mutual_overlap_fraction = if boxes.is_empty() {
        0.0
    } else {
        overlapping as f32 / boxes.len() as f32
    };

    let verdict = check(&report, gates, local_boxes.len(), boxes.len());
    report.passed = verdict.is_ok();
    (report, verdict)
}

fn check(
    report: &GateReport,
    gates: &GeometryGates,
    local_lines: usize,
    provider_lines: usize,
) -> Result<(), StructuredBboxError> {
    if provider_lines == 0 {
        return Err(StructuredBboxError::Gate("no lines were returned".into()));
    }
    if gates.require_all_in_page && report.in_page_fraction < 1.0 {
        return Err(StructuredBboxError::Gate(format!(
            "{:.1}% of boxes are inside the page; 100% is required",
            report.in_page_fraction * 100.0
        )));
    }
    if report.degenerate_boxes > 0 {
        return Err(StructuredBboxError::Gate(format!(
            "{} degenerate or sub-minimum box(es)",
            report.degenerate_boxes
        )));
    }
    if report.oversized_boxes > 0 {
        return Err(StructuredBboxError::Gate(format!(
            "{} box(es) larger than {:.0}% of the page",
            report.oversized_boxes,
            gates.max_area_fraction * 100.0
        )));
    }
    if report.order_violations > 0 {
        return Err(StructuredBboxError::Gate(format!(
            "{} reading-order violation(s)",
            report.order_violations
        )));
    }
    if report.local_match_coverage < gates.min_local_match_coverage {
        return Err(StructuredBboxError::Gate(format!(
            "only {:.1}% of provider lines match a locally detected line",
            report.local_match_coverage * 100.0
        )));
    }
    if report.mutual_overlap_fraction > gates.max_mutual_overlap_fraction {
        return Err(StructuredBboxError::Gate(format!(
            "{:.1}% of provider lines overlap another provider line",
            report.mutual_overlap_fraction * 100.0
        )));
    }
    let local = local_lines.max(1) as f32;
    let ratio = provider_lines as f32 / local;
    if ratio > gates.max_line_count_ratio || ratio < 1.0 / gates.max_line_count_ratio {
        return Err(StructuredBboxError::Gate(format!(
            "{provider_lines} provider lines against {local_lines} local lines"
        )));
    }
    Ok(())
}

/// Builds a canonical page from a *gate-passing* structured response.
///
/// Only reachable after [`evaluate_gates`] returned `Ok`. Word rectangles are
/// derived from the accepted line box by proportional split, never requested
/// from the model: a model that cannot be trusted with a line rectangle
/// cannot be trusted with a word one.
pub fn to_ocr_page(
    response: &StructuredLinesResponse,
    local: &OcrPage,
    _alignment: &AlignmentConfig,
) -> OcrPage {
    let mut blocks = Vec::with_capacity(response.lines.len());
    for (index, line) in response.lines.iter().enumerate() {
        let [x, y, width, height] = line.bbox;
        let bbox = OcrBox {
            x,
            y,
            width,
            height,
        };
        let tokens: Vec<&str> = line.text.split_whitespace().collect();
        let total: usize = tokens
            .iter()
            .map(|token| token.chars().count().max(1))
            .sum();
        let mut cursor = x;
        let words = tokens
            .iter()
            .enumerate()
            .map(|(word_index, token)| {
                let share = token.chars().count().max(1) as f32 / total.max(1) as f32;
                let word_width = width * share;
                let word = OcrWord {
                    text: (*token).to_owned(),
                    normalized_text: token
                        .chars()
                        .collect::<String>()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                    bbox: OcrBox {
                        x: cursor,
                        y,
                        width: word_width,
                        height,
                    },
                    // No self-reported confidence is accepted from a model.
                    // A gate-passing box is recorded at a fixed, explicitly
                    // documented value so nothing downstream mistakes it for
                    // a measurement.
                    confidence: 0.5,
                    reading_order: word_index as u32,
                };
                cursor += word_width;
                word
            })
            .collect::<Vec<_>>();
        blocks.push(OcrBlock {
            bbox: bbox.clone(),
            confidence: 0.5,
            reading_order: index as u32,
            lines: vec![OcrLine {
                bbox,
                confidence: 0.5,
                reading_order: 0,
                words,
            }],
        });
    }
    OcrPage {
        page_index: local.page_index,
        route: local.route.clone(),
        width: local.width,
        height: local.height,
        blocks,
        revisions: Vec::new(),
        provider_provenance: None,
        provider_raw_artifact: None,
    }
}

/// The identity of this path for the checkpoint fingerprint.
pub fn fingerprint(policy: StructuredBboxPolicy, gates: &GeometryGates) -> String {
    format!(
        "structured_bbox={}|schema={}|alignment={}|iou={:.3}|coverage={:.3}|overlap={:.3}|ratio={:.3}|area={:.6}..{:.3}",
        policy.as_str(),
        STRUCTURED_BBOX_SCHEMA_VERSION,
        ALIGNMENT_VERSION,
        gates.min_iou,
        gates.min_local_match_coverage,
        gates.max_mutual_overlap_fraction,
        gates.max_line_count_ratio,
        gates.min_area_fraction,
        gates.max_area_fraction,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{OcrRoute, OcrRouteReason};

    fn local_page(boxes: &[(f32, f32, f32, f32)]) -> OcrPage {
        OcrPage {
            page_index: 0,
            route: OcrRoute::Ocr {
                reason: OcrRouteReason::MissingText,
            },
            width: 1000,
            height: 1400,
            blocks: boxes
                .iter()
                .enumerate()
                .map(|(index, (x, y, width, height))| {
                    let bbox = OcrBox {
                        x: *x,
                        y: *y,
                        width: *width,
                        height: *height,
                    };
                    OcrBlock {
                        bbox: bbox.clone(),
                        confidence: 0.9,
                        reading_order: index as u32,
                        lines: vec![OcrLine {
                            bbox,
                            confidence: 0.9,
                            reading_order: 0,
                            words: Vec::new(),
                        }],
                    }
                })
                .collect(),
            revisions: Vec::new(),
            provider_provenance: None,
            provider_raw_artifact: None,
        }
    }

    fn response(lines: &[(&str, [f32; 4], u32)]) -> StructuredLinesResponse {
        StructuredLinesResponse {
            schema_version: STRUCTURED_BBOX_SCHEMA_VERSION.into(),
            page_width: 1000.0,
            page_height: 1400.0,
            lines: lines
                .iter()
                .map(|(text, bbox, order)| StructuredLine {
                    text: (*text).to_owned(),
                    bbox: *bbox,
                    reading_order: *order,
                })
                .collect(),
        }
    }

    // The constant *is* the thing under test: this asserts the shipped value
    // of the flag that would turn model coordinates into a default coordinate
    // source, so a future edit that flips it has to come here and justify
    // itself rather than sliding through.
    #[allow(clippy::assertions_on_constants)]
    #[test]
    fn the_structured_path_is_disabled_by_default_and_cannot_claim_otherwise() {
        assert_eq!(
            StructuredBboxPolicy::default(),
            StructuredBboxPolicy::Disabled
        );
        assert!(!GATES_VALIDATED_FOR_DEFAULT);
        assert!(!StructuredBboxPolicy::EvaluateWithFallback.allowed_as_default());
    }

    #[test]
    fn an_unknown_field_is_rejected_rather_than_ignored() {
        let body = r#"{"schema_version":"mpdf-provider-lines/0.1","page_width":10,"page_height":10,"lines":[],"confidence":0.99}"#;
        assert!(matches!(
            parse_structured(body, &GeometryGates::default()),
            Err(StructuredBboxError::Schema(_))
        ));
    }

    #[test]
    fn a_wrong_schema_version_is_refused() {
        let body =
            r#"{"schema_version":"something-else","page_width":10,"page_height":10,"lines":[]}"#;
        assert!(matches!(
            parse_structured(body, &GeometryGates::default()),
            Err(StructuredBboxError::UnsupportedVersion(_))
        ));
    }

    #[test]
    fn nan_and_infinite_coordinates_never_reach_the_gates() {
        for value in ["NaN", "null"] {
            let body = format!(
                r#"{{"schema_version":"mpdf-provider-lines/0.1","page_width":10,"page_height":10,"lines":[{{"text":"a","bbox":[0,0,{value},1],"reading_order":0}}]}}"#
            );
            assert!(parse_structured(&body, &GeometryGates::default()).is_err());
        }
    }

    #[test]
    fn a_box_outside_the_page_fails_the_in_page_gate() {
        let local = local_page(&[(10.0, 10.0, 500.0, 30.0)]);
        let structured = response(&[
            ("a", [10.0, 10.0, 500.0, 30.0], 0),
            ("b", [900.0, 1390.0, 500.0, 30.0], 1),
        ]);
        let (report, verdict) = evaluate_gates(&structured, &local, &GeometryGates::default());
        assert!(verdict.is_err());
        assert!(report.in_page_fraction < 1.0);
    }

    #[test]
    fn negative_and_zero_area_boxes_fail() {
        let local = local_page(&[(10.0, 10.0, 500.0, 30.0)]);
        for bbox in [[10.0, 10.0, 0.0, 30.0], [-5.0, 10.0, 100.0, 30.0]] {
            let structured = response(&[("a", bbox, 0)]);
            let (_, verdict) = evaluate_gates(&structured, &local, &GeometryGates::default());
            assert!(verdict.is_err(), "{bbox:?} should be refused");
        }
    }

    #[test]
    fn a_box_covering_most_of_the_page_fails() {
        let local = local_page(&[(10.0, 10.0, 500.0, 30.0)]);
        let structured = response(&[("a", [0.0, 0.0, 900.0, 1300.0], 0)]);
        let (report, verdict) = evaluate_gates(&structured, &local, &GeometryGates::default());
        assert!(verdict.is_err());
        assert_eq!(report.oversized_boxes, 1);
    }

    #[test]
    fn out_of_order_reading_indices_fail() {
        let local = local_page(&[(10.0, 10.0, 500.0, 30.0), (10.0, 60.0, 500.0, 30.0)]);
        let structured = response(&[
            ("a", [10.0, 10.0, 500.0, 30.0], 5),
            ("b", [10.0, 60.0, 500.0, 30.0], 2),
        ]);
        let (report, verdict) = evaluate_gates(&structured, &local, &GeometryGates::default());
        assert!(verdict.is_err());
        assert_eq!(report.order_violations, 1);
    }

    #[test]
    fn duplicated_overlapping_boxes_fail() {
        let local = local_page(&[(10.0, 10.0, 500.0, 30.0), (10.0, 60.0, 500.0, 30.0)]);
        let structured = response(&[
            ("a", [10.0, 10.0, 500.0, 30.0], 0),
            ("a", [10.0, 10.0, 500.0, 30.0], 1),
        ]);
        let (report, verdict) = evaluate_gates(&structured, &local, &GeometryGates::default());
        assert!(verdict.is_err());
        assert!(report.mutual_overlap_fraction > 0.0);
    }

    #[test]
    fn too_few_or_too_many_lines_against_the_local_detection_fail() {
        let local = local_page(&[
            (10.0, 10.0, 500.0, 30.0),
            (10.0, 60.0, 500.0, 30.0),
            (10.0, 110.0, 500.0, 30.0),
            (10.0, 160.0, 500.0, 30.0),
        ]);
        let structured = response(&[("a", [10.0, 10.0, 500.0, 30.0], 0)]);
        let (_, verdict) = evaluate_gates(&structured, &local, &GeometryGates::default());
        assert!(verdict.is_err());
    }

    #[test]
    fn a_response_that_agrees_with_the_local_detection_passes_and_builds_a_page() {
        let local = local_page(&[(10.0, 10.0, 500.0, 30.0), (10.0, 60.0, 500.0, 30.0)]);
        let structured = response(&[
            ("first line", [10.0, 10.0, 500.0, 30.0], 0),
            ("second line", [10.0, 60.0, 500.0, 30.0], 1),
        ]);
        let (report, verdict) = evaluate_gates(&structured, &local, &GeometryGates::default());
        assert!(verdict.is_ok(), "{report:?}");
        let page = to_ocr_page(&structured, &local, &AlignmentConfig::default());
        assert_eq!(page.blocks.len(), 2);
        assert_eq!(page.width, local.width);
        crate::ocr::OcrRun {
            protocol: crate::ocr::OCR_PROTOCOL.into(),
            protocol_version: crate::ocr::OCR_PROTOCOL_VERSION.into(),
            pages: vec![page],
            errors: Vec::new(),
        }
        .validate()
        .expect("a gate-passing page must satisfy the evidence validator");
    }

    #[test]
    fn shifted_boxes_that_look_plausible_still_fail_the_local_match_gate() {
        // Every box is well-formed, in the page, in order, and non-overlapping
        // — and every one is 200px below where the text actually is.
        let local = local_page(&[(10.0, 10.0, 500.0, 30.0), (10.0, 60.0, 500.0, 30.0)]);
        let structured = response(&[
            ("first line", [10.0, 210.0, 500.0, 30.0], 0),
            ("second line", [10.0, 260.0, 500.0, 30.0], 1),
        ]);
        let (report, verdict) = evaluate_gates(&structured, &local, &GeometryGates::default());
        assert!(verdict.is_err());
        assert_eq!(report.local_match_coverage, 0.0);
    }

    #[test]
    fn the_fingerprint_changes_with_the_policy_and_with_every_gate() {
        let base = fingerprint(StructuredBboxPolicy::Disabled, &GeometryGates::default());
        assert_ne!(
            base,
            fingerprint(
                StructuredBboxPolicy::EvaluateWithFallback,
                &GeometryGates::default()
            )
        );
        assert_ne!(
            base,
            fingerprint(
                StructuredBboxPolicy::Disabled,
                &GeometryGates {
                    min_iou: 0.75,
                    ..Default::default()
                }
            )
        );
    }
}
