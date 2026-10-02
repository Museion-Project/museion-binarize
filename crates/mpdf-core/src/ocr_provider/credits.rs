//! The M PDF Credits protocol: reservation, execution, settlement, refund.
//!
//! # What this is and, more importantly, what it is not
//!
//! This module is a complete, versioned *client* protocol and state machine
//! for brokered cloud OCR, plus a deterministic fake backend that the tests
//! drive through every transition. It is **not** a payment system, and this
//! repository contains no production service that implements the other side
//! of it. [`PRODUCTION_BACKEND`] is `None` and
//! [`release_blockers`] enumerates exactly what is missing. Both front ends
//! read those and must refuse to describe Credits as available for real work.
//! Shipping a convincing but unbacked billing flow would be worse than
//! shipping none.
//!
//! # Why the client holds no provider key
//!
//! The entire point of brokered execution is that the platform's model
//! credential never reaches a user's machine. The client therefore holds a
//! *short-lived, job-scoped token* obtained per session, and the only
//! authority it has is over its own reservations. A client that could read
//! the platform's Gemini key would be a client that could spend the
//! platform's money, and every desktop binary would be a copy of it.
//!
//! # Why reservations and not "charge on success"
//!
//! A 400-page book is 400 billable operations. Charging as they complete
//! means a user can start a job they cannot pay to finish, and a crash
//! halfway leaves an ambiguous ledger. Reserving the estimate up front makes
//! insufficient balance a *pre-flight* failure — before a single page image
//! is uploaded — and makes the failure path arithmetic trivial: whatever was
//! not spent is released.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CREDITS_PROTOCOL: &str = "mpdf-credits";
pub const CREDITS_PROTOCOL_VERSION: &str = "0.1";

/// The production Credits service. There is not one.
///
/// This is the single source of truth both front ends consult before they
/// describe the mode. Wiring one up means setting this *and* clearing the
/// blockers below, not one or the other.
pub const PRODUCTION_BACKEND: Option<&str> = None;

/// Everything that must exist before Credits may be offered as a real
/// product. Rendered verbatim by `mpdf run --help`, the desktop provider
/// picker, and the release-readiness report.
pub fn release_blockers() -> Vec<&'static str> {
    vec![
        "no production Credits service is deployed; only the in-process fake backend exists",
        "no payment provider is integrated and no real credits can be purchased",
        "server-side custody of the platform model credential is unimplemented",
        "receipt signing keys are not provisioned, so signatures are verified against a test secret only",
        "the privacy policy, data-retention window and deletion endpoint for uploaded page images are not published",
    ]
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CreditsError {
    #[error("insufficient credits: {available} available, {required} required")]
    InsufficientCredits { available: u64, required: u64 },
    #[error("invalid credits state transition: {from} -> {to}")]
    InvalidTransition {
        from: &'static str,
        to: &'static str,
    },
    #[error("unknown reservation")]
    UnknownReservation,
    #[error("credits receipt failed verification: {0}")]
    ReceiptInvalid(String),
    #[error("credits backend unavailable: {0}")]
    Unavailable(String),
    #[error("credits protocol {0} is not supported")]
    UnsupportedProtocol(String),
}

/// The lifecycle. Every transition is explicit; there is no path from
/// `Executing` straight to `Reserved`, and no path out of a terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationState {
    Draft,
    Reserved,
    Executing,
    /// Terminal. Charged what was used, released the rest.
    Settled,
    /// Terminal. The user stopped it; used pages still charged.
    Cancelled,
    /// Terminal. The provider or transport failed; used pages still charged.
    Failed,
    /// Terminal. A settled reservation that was later reversed in full.
    Refunded,
}

impl ReservationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Reserved => "reserved",
            Self::Executing => "executing",
            Self::Settled => "settled",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Refunded => "refunded",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Settled | Self::Cancelled | Self::Failed | Self::Refunded
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CreditsBalance {
    pub available: u64,
    pub reserved: u64,
}

/// What the client asks the service to hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReservationRequest {
    pub protocol: String,
    pub protocol_version: String,
    /// Binds the reservation to one document, so a token issued for a
    /// 3-page test cannot be replayed against a 900-page volume.
    pub document_sha256: String,
    pub first_page_index: u32,
    pub page_count: u32,
    pub provider_config_digest: String,
    pub estimated_credits: u64,
    /// The ceiling the user authorized. The service must refuse to hold more.
    pub max_credits: u64,
    /// Stable across retries of the *same* reservation attempt.
    pub idempotency_key: String,
}

impl ReservationRequest {
    pub fn new(
        document_sha256: impl Into<String>,
        first_page_index: u32,
        page_count: u32,
        provider_config_digest: impl Into<String>,
        estimated_credits: u64,
        max_credits: u64,
    ) -> Self {
        let document_sha256 = document_sha256.into();
        let provider_config_digest = provider_config_digest.into();
        // Derived, never random: a retried reservation for identical work
        // must present an identical key or the retry becomes a second hold.
        let idempotency_key = super::hex(&Sha256::digest(
            format!(
                "{CREDITS_PROTOCOL}/{CREDITS_PROTOCOL_VERSION}|{document_sha256}|{first_page_index}|{page_count}|{provider_config_digest}|{max_credits}"
            )
            .as_bytes(),
        ));
        Self {
            protocol: CREDITS_PROTOCOL.into(),
            protocol_version: CREDITS_PROTOCOL_VERSION.into(),
            document_sha256,
            first_page_index,
            page_count,
            provider_config_digest,
            estimated_credits,
            max_credits,
            idempotency_key,
        }
    }

    pub fn validate(&self) -> Result<(), CreditsError> {
        if self.protocol != CREDITS_PROTOCOL || self.protocol_version != CREDITS_PROTOCOL_VERSION {
            return Err(CreditsError::UnsupportedProtocol(format!(
                "{}/{}",
                self.protocol, self.protocol_version
            )));
        }
        if self.estimated_credits > self.max_credits {
            return Err(CreditsError::InsufficientCredits {
                available: self.max_credits,
                required: self.estimated_credits,
            });
        }
        Ok(())
    }
}

/// The service's answer. Everything a client needs to audit the job later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reservation {
    pub reservation_id: String,
    pub job_id: String,
    pub held_credits: u64,
    pub state: ReservationState,
    pub model: String,
    pub model_version: String,
    /// Opaque, short-lived, job-scoped. Never the platform's provider key,
    /// and never persisted to the workspace.
    pub job_token_reference: String,
}

/// One page's charge, recorded against a reservation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageCharge {
    pub page_index: u32,
    pub idempotency_key: String,
    pub credits: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// The signed statement the client verifies before believing any of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settlement {
    pub protocol: String,
    pub protocol_version: String,
    pub reservation_id: String,
    pub job_id: String,
    pub model: String,
    pub model_version: String,
    pub state: ReservationState,
    pub pages_charged: u32,
    pub charged_credits: u64,
    pub refunded_credits: u64,
    pub released_credits: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// HMAC-SHA256 over [`Settlement::signing_payload`], hex-encoded. The
    /// client refuses a settlement it cannot verify rather than trusting a
    /// number that decides what the user was charged.
    pub signature: String,
}

impl Settlement {
    /// Canonical, order-independent payload. Field order here is the
    /// protocol; changing it is a protocol version bump.
    pub fn signing_payload(&self) -> String {
        format!(
            "{}/{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.protocol,
            self.protocol_version,
            self.reservation_id,
            self.job_id,
            self.model,
            self.model_version,
            self.state.as_str(),
            self.pages_charged,
            self.charged_credits,
            self.refunded_credits,
            self.released_credits,
            self.input_tokens,
            self.output_tokens,
        )
    }

    pub fn sign(&mut self, secret: &[u8]) {
        self.signature = hmac_sha256_hex(secret, self.signing_payload().as_bytes());
    }

    /// Constant-time-ish verification. The payload is not secret, so the
    /// comparison only needs to avoid an early-exit oracle on the tag.
    pub fn verify(&self, secret: &[u8]) -> Result<(), CreditsError> {
        let expected = hmac_sha256_hex(secret, self.signing_payload().as_bytes());
        let equal = expected.len() == self.signature.len()
            && expected
                .bytes()
                .zip(self.signature.bytes())
                .fold(0_u8, |accumulator, (a, b)| accumulator | (a ^ b))
                == 0;
        if equal {
            Ok(())
        } else {
            Err(CreditsError::ReceiptInvalid(
                "signature does not match the settlement contents".into(),
            ))
        }
    }
}

/// HMAC-SHA256, hex. Written out rather than pulled in as a dependency: it is
/// twelve lines, and the alternative is another crate in the audited
/// dependency set for one call site.
fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut normalized = [0_u8; BLOCK];
    if key.len() > BLOCK {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_key = [0x36_u8; BLOCK];
    let mut outer_key = [0x5c_u8; BLOCK];
    for index in 0..BLOCK {
        inner_key[index] ^= normalized[index];
        outer_key[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_key);
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_key);
    outer.update(inner);
    super::hex(&outer.finalize())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    Cancelled,
    Failed,
}

/// The transport a real service would implement.
pub trait CreditsBackend {
    fn balance(&self) -> Result<CreditsBalance, CreditsError>;
    fn reserve(&mut self, request: &ReservationRequest) -> Result<Reservation, CreditsError>;
    /// Records one page. Must be idempotent on
    /// [`PageCharge::idempotency_key`]: a retried page never bills twice.
    fn record_page(
        &mut self,
        reservation_id: &str,
        charge: &PageCharge,
    ) -> Result<(), CreditsError>;
    fn settle(&mut self, reservation_id: &str) -> Result<Settlement, CreditsError>;
    fn release(
        &mut self,
        reservation_id: &str,
        reason: ReleaseReason,
    ) -> Result<Settlement, CreditsError>;
    fn refund(&mut self, reservation_id: &str) -> Result<Settlement, CreditsError>;
}

/// In-process reference implementation of the whole state machine.
///
/// This is the mock server the tests drive. It is deliberately in the core so
/// the state machine has exactly one implementation: a real backend replaces
/// the *transport*, not the rules.
#[derive(Debug)]
pub struct FakeCreditsBackend {
    balance: CreditsBalance,
    reservations: BTreeMap<String, Held>,
    /// Reservation ids already issued for a given idempotency key.
    issued: BTreeMap<String, String>,
    secret: Vec<u8>,
    next_id: u64,
    model: String,
    model_version: String,
}

#[derive(Debug, Clone)]
struct Held {
    request: ReservationRequest,
    state: ReservationState,
    held: u64,
    charges: BTreeMap<String, PageCharge>,
    settlement: Option<Settlement>,
}

impl Held {
    fn used(&self) -> u64 {
        self.charges
            .values()
            .fold(0_u64, |total, charge| total.saturating_add(charge.credits))
    }
}

impl FakeCreditsBackend {
    pub fn new(available: u64, secret: impl Into<Vec<u8>>) -> Self {
        Self {
            balance: CreditsBalance {
                available,
                reserved: 0,
            },
            reservations: BTreeMap::new(),
            issued: BTreeMap::new(),
            secret: secret.into(),
            next_id: 1,
            model: "gemini-3.7-flash".into(),
            model_version: "2026-08".into(),
        }
    }

    pub fn secret(&self) -> &[u8] {
        &self.secret
    }

    fn finish(
        &mut self,
        reservation_id: &str,
        state: ReservationState,
    ) -> Result<Settlement, CreditsError> {
        let held = self
            .reservations
            .get_mut(reservation_id)
            .ok_or(CreditsError::UnknownReservation)?;
        if let Some(existing) = &held.settlement {
            // Terminal states are idempotent: a retried settle returns the
            // statement that was already produced instead of moving money a
            // second time.
            return Ok(existing.clone());
        }
        if held.state.is_terminal() {
            return Err(CreditsError::InvalidTransition {
                from: held.state.as_str(),
                to: state.as_str(),
            });
        }
        let used = held.used().min(held.held);
        let released = held.held.saturating_sub(used);
        let mut settlement = Settlement {
            protocol: CREDITS_PROTOCOL.into(),
            protocol_version: CREDITS_PROTOCOL_VERSION.into(),
            reservation_id: reservation_id.to_owned(),
            job_id: held.request.idempotency_key.clone(),
            model: self.model.clone(),
            model_version: self.model_version.clone(),
            state,
            pages_charged: held.charges.len() as u32,
            charged_credits: used,
            refunded_credits: 0,
            released_credits: released,
            input_tokens: held
                .charges
                .values()
                .fold(0, |total, charge| total + charge.input_tokens),
            output_tokens: held
                .charges
                .values()
                .fold(0, |total, charge| total + charge.output_tokens),
            signature: String::new(),
        };
        settlement.sign(&self.secret);
        held.state = state;
        held.settlement = Some(settlement.clone());
        self.balance.reserved = self.balance.reserved.saturating_sub(held.held);
        self.balance.available = self.balance.available.saturating_add(released);
        Ok(settlement)
    }
}

impl CreditsBackend for FakeCreditsBackend {
    fn balance(&self) -> Result<CreditsBalance, CreditsError> {
        Ok(self.balance)
    }

    fn reserve(&mut self, request: &ReservationRequest) -> Result<Reservation, CreditsError> {
        request.validate()?;
        if let Some(existing) = self.issued.get(&request.idempotency_key) {
            let held = &self.reservations[existing];
            return Ok(Reservation {
                reservation_id: existing.clone(),
                job_id: request.idempotency_key.clone(),
                held_credits: held.held,
                state: held.state,
                model: self.model.clone(),
                model_version: self.model_version.clone(),
                job_token_reference: format!("job-token:{existing}"),
            });
        }
        let hold = request.estimated_credits.min(request.max_credits);
        if self.balance.available < hold {
            return Err(CreditsError::InsufficientCredits {
                available: self.balance.available,
                required: hold,
            });
        }
        let reservation_id = format!("res-{:06}", self.next_id);
        self.next_id += 1;
        self.balance.available -= hold;
        self.balance.reserved += hold;
        self.reservations.insert(
            reservation_id.clone(),
            Held {
                request: request.clone(),
                state: ReservationState::Reserved,
                held: hold,
                charges: BTreeMap::new(),
                settlement: None,
            },
        );
        self.issued
            .insert(request.idempotency_key.clone(), reservation_id.clone());
        Ok(Reservation {
            job_id: request.idempotency_key.clone(),
            held_credits: hold,
            state: ReservationState::Reserved,
            model: self.model.clone(),
            model_version: self.model_version.clone(),
            job_token_reference: format!("job-token:{reservation_id}"),
            reservation_id,
        })
    }

    fn record_page(
        &mut self,
        reservation_id: &str,
        charge: &PageCharge,
    ) -> Result<(), CreditsError> {
        let held = self
            .reservations
            .get_mut(reservation_id)
            .ok_or(CreditsError::UnknownReservation)?;
        if held.state.is_terminal() {
            return Err(CreditsError::InvalidTransition {
                from: held.state.as_str(),
                to: ReservationState::Executing.as_str(),
            });
        }
        held.state = ReservationState::Executing;
        // The whole idempotency guarantee, in one line: a retry with the same
        // derived key overwrites its own record rather than adding one.
        held.charges
            .insert(charge.idempotency_key.clone(), charge.clone());
        if held.used() > held.held {
            return Err(CreditsError::InsufficientCredits {
                available: held.held,
                required: held.used(),
            });
        }
        Ok(())
    }

    fn settle(&mut self, reservation_id: &str) -> Result<Settlement, CreditsError> {
        self.finish(reservation_id, ReservationState::Settled)
    }

    fn release(
        &mut self,
        reservation_id: &str,
        reason: ReleaseReason,
    ) -> Result<Settlement, CreditsError> {
        let state = match reason {
            ReleaseReason::Cancelled => ReservationState::Cancelled,
            ReleaseReason::Failed => ReservationState::Failed,
        };
        self.finish(reservation_id, state)
    }

    fn refund(&mut self, reservation_id: &str) -> Result<Settlement, CreditsError> {
        let secret = self.secret.clone();
        let held = self
            .reservations
            .get_mut(reservation_id)
            .ok_or(CreditsError::UnknownReservation)?;
        let Some(previous) = held.settlement.clone() else {
            return Err(CreditsError::InvalidTransition {
                from: held.state.as_str(),
                to: ReservationState::Refunded.as_str(),
            });
        };
        if held.state == ReservationState::Refunded {
            return Ok(previous);
        }
        let mut settlement = Settlement {
            state: ReservationState::Refunded,
            charged_credits: 0,
            refunded_credits: previous.charged_credits,
            ..previous
        };
        settlement.sign(&secret);
        held.state = ReservationState::Refunded;
        held.settlement = Some(settlement.clone());
        self.balance.available = self
            .balance
            .available
            .saturating_add(settlement.refunded_credits);
        Ok(settlement)
    }
}

/// Derives the per-page idempotency key.
///
/// A key must be a pure function of *what work it is*, never of when it was
/// attempted. Any random or time-based component turns a retry into a second
/// charge, which is the single most expensive bug this protocol can have.
pub fn page_idempotency_key(
    document_sha256: &str,
    page_index: u32,
    page_image_sha256: &str,
    provider_config_digest: &str,
) -> String {
    super::hex(&Sha256::digest(
        format!(
            "{CREDITS_PROTOCOL}/{CREDITS_PROTOCOL_VERSION}|{document_sha256}|{page_index}|{page_image_sha256}|{provider_config_digest}"
        )
        .as_bytes(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(pages: u32, estimate: u64, max: u64) -> ReservationRequest {
        ReservationRequest::new("a".repeat(64), 0, pages, "config-digest", estimate, max)
    }

    fn charge(page: u32, credits: u64) -> PageCharge {
        PageCharge {
            page_index: page,
            idempotency_key: page_idempotency_key(
                &"a".repeat(64),
                page,
                &"b".repeat(64),
                "config-digest",
            ),
            credits,
            input_tokens: 1_000,
            output_tokens: 400,
        }
    }

    #[test]
    fn there_is_no_production_backend_and_the_blockers_are_explicit() {
        assert!(PRODUCTION_BACKEND.is_none());
        assert!(release_blockers().len() >= 4);
        assert!(release_blockers()
            .iter()
            .any(|blocker| blocker.contains("payment")));
    }

    #[test]
    fn a_reservation_holds_the_estimate_and_settles_what_was_used() {
        let mut backend = FakeCreditsBackend::new(1_000, b"test-secret".to_vec());
        let reservation = backend.reserve(&request(10, 100, 120)).unwrap();
        assert_eq!(reservation.held_credits, 100);
        assert_eq!(backend.balance().unwrap().available, 900);
        for page in 0..10 {
            backend
                .record_page(&reservation.reservation_id, &charge(page, 7))
                .unwrap();
        }
        let settlement = backend.settle(&reservation.reservation_id).unwrap();
        assert_eq!(settlement.charged_credits, 70);
        assert_eq!(settlement.released_credits, 30);
        assert_eq!(settlement.state, ReservationState::Settled);
        assert_eq!(backend.balance().unwrap().available, 930);
        assert_eq!(backend.balance().unwrap().reserved, 0);
    }

    #[test]
    fn a_retried_page_does_not_bill_twice() {
        let mut backend = FakeCreditsBackend::new(1_000, b"s".to_vec());
        let reservation = backend.reserve(&request(3, 30, 30)).unwrap();
        let page = charge(1, 9);
        backend
            .record_page(&reservation.reservation_id, &page)
            .unwrap();
        backend
            .record_page(&reservation.reservation_id, &page)
            .unwrap();
        backend
            .record_page(&reservation.reservation_id, &page)
            .unwrap();
        let settlement = backend.settle(&reservation.reservation_id).unwrap();
        assert_eq!(settlement.pages_charged, 1);
        assert_eq!(settlement.charged_credits, 9);
    }

    #[test]
    fn a_page_key_depends_only_on_the_work_not_on_the_attempt() {
        let first = page_idempotency_key("doc", 4, "img", "cfg");
        let second = page_idempotency_key("doc", 4, "img", "cfg");
        assert_eq!(first, second);
        assert_ne!(first, page_idempotency_key("doc", 5, "img", "cfg"));
        assert_ne!(first, page_idempotency_key("doc", 4, "img2", "cfg"));
        assert_ne!(first, page_idempotency_key("doc", 4, "img", "cfg2"));
    }

    #[test]
    fn a_retried_reservation_reuses_the_hold_instead_of_taking_a_second_one() {
        let mut backend = FakeCreditsBackend::new(100, b"s".to_vec());
        let first = backend.reserve(&request(5, 50, 50)).unwrap();
        let second = backend.reserve(&request(5, 50, 50)).unwrap();
        assert_eq!(first.reservation_id, second.reservation_id);
        assert_eq!(backend.balance().unwrap().available, 50);
    }

    #[test]
    fn an_insufficient_balance_fails_before_anything_is_uploaded() {
        let mut backend = FakeCreditsBackend::new(10, b"s".to_vec());
        assert_eq!(
            backend.reserve(&request(100, 400, 400)),
            Err(CreditsError::InsufficientCredits {
                available: 10,
                required: 400
            })
        );
        assert_eq!(backend.balance().unwrap().reserved, 0);
    }

    #[test]
    fn cancelling_charges_completed_pages_and_releases_the_rest() {
        let mut backend = FakeCreditsBackend::new(500, b"s".to_vec());
        let reservation = backend.reserve(&request(10, 100, 100)).unwrap();
        backend
            .record_page(&reservation.reservation_id, &charge(0, 8))
            .unwrap();
        backend
            .record_page(&reservation.reservation_id, &charge(1, 8))
            .unwrap();
        let settlement = backend
            .release(&reservation.reservation_id, ReleaseReason::Cancelled)
            .unwrap();
        assert_eq!(settlement.state, ReservationState::Cancelled);
        assert_eq!(settlement.charged_credits, 16);
        assert_eq!(settlement.released_credits, 84);
        assert_eq!(backend.balance().unwrap().available, 484);
    }

    #[test]
    fn a_failed_job_releases_its_unused_hold() {
        let mut backend = FakeCreditsBackend::new(500, b"s".to_vec());
        let reservation = backend.reserve(&request(10, 100, 100)).unwrap();
        let settlement = backend
            .release(&reservation.reservation_id, ReleaseReason::Failed)
            .unwrap();
        assert_eq!(settlement.state, ReservationState::Failed);
        assert_eq!(settlement.charged_credits, 0);
        assert_eq!(settlement.released_credits, 100);
        assert_eq!(backend.balance().unwrap().available, 500);
    }

    #[test]
    fn settling_twice_returns_the_same_statement_and_moves_nothing() {
        let mut backend = FakeCreditsBackend::new(500, b"s".to_vec());
        let reservation = backend.reserve(&request(4, 40, 40)).unwrap();
        backend
            .record_page(&reservation.reservation_id, &charge(0, 10))
            .unwrap();
        let first = backend.settle(&reservation.reservation_id).unwrap();
        let balance = backend.balance().unwrap();
        let second = backend.settle(&reservation.reservation_id).unwrap();
        assert_eq!(first, second);
        assert_eq!(backend.balance().unwrap(), balance);
    }

    #[test]
    fn a_refund_reverses_a_settlement_exactly_once() {
        let mut backend = FakeCreditsBackend::new(500, b"s".to_vec());
        let reservation = backend.reserve(&request(4, 40, 40)).unwrap();
        backend
            .record_page(&reservation.reservation_id, &charge(0, 10))
            .unwrap();
        backend.settle(&reservation.reservation_id).unwrap();
        let refunded = backend.refund(&reservation.reservation_id).unwrap();
        assert_eq!(refunded.state, ReservationState::Refunded);
        assert_eq!(refunded.refunded_credits, 10);
        let balance = backend.balance().unwrap();
        assert_eq!(
            backend.refund(&reservation.reservation_id).unwrap(),
            refunded
        );
        assert_eq!(backend.balance().unwrap(), balance);
    }

    #[test]
    fn recording_a_page_after_a_terminal_state_is_refused() {
        let mut backend = FakeCreditsBackend::new(500, b"s".to_vec());
        let reservation = backend.reserve(&request(4, 40, 40)).unwrap();
        backend.settle(&reservation.reservation_id).unwrap();
        assert!(matches!(
            backend.record_page(&reservation.reservation_id, &charge(0, 1)),
            Err(CreditsError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn a_settlement_is_rejected_when_its_numbers_were_edited() {
        let mut backend = FakeCreditsBackend::new(500, b"the-signing-secret".to_vec());
        let reservation = backend.reserve(&request(4, 40, 40)).unwrap();
        backend
            .record_page(&reservation.reservation_id, &charge(0, 10))
            .unwrap();
        let settlement = backend.settle(&reservation.reservation_id).unwrap();
        settlement
            .verify(backend.secret())
            .expect("genuine statement verifies");

        let tampered = Settlement {
            charged_credits: settlement.charged_credits + 500,
            ..settlement.clone()
        };
        assert!(tampered.verify(backend.secret()).is_err());
        // A different signing key is also refused, so a settlement from
        // another tenant cannot be replayed here.
        assert!(settlement.verify(b"another-secret").is_err());
    }

    #[test]
    fn a_reservation_over_the_authorized_ceiling_is_refused_by_the_request_itself() {
        let over = ReservationRequest::new("a".repeat(64), 0, 10, "cfg", 500, 100);
        assert!(matches!(
            over.validate(),
            Err(CreditsError::InsufficientCredits { .. })
        ));
    }

    #[test]
    fn no_terminal_state_can_be_left_except_by_a_refund() {
        for state in [
            ReservationState::Settled,
            ReservationState::Cancelled,
            ReservationState::Failed,
            ReservationState::Refunded,
        ] {
            assert!(state.is_terminal(), "{}", state.as_str());
        }
        assert!(!ReservationState::Reserved.is_terminal());
        assert!(!ReservationState::Executing.is_terminal());
    }
}
