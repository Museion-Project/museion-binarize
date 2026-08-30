//! Provider selection and credential commands.
//!
//! # The rule that shapes this file
//!
//! A key crosses the IPC boundary exactly once, in one direction: from the
//! password field into [`store_model_provider_credential`], which hands it
//! straight to the OS credential store. Nothing here ever returns one, and no
//! other DTO in this app has a field that could carry one. That is why the
//! settings screen can offer "replace" but not "show": there is nothing to
//! show, by construction rather than by policy.

use mpdf_api_client::cloud_ocr::{
    CloudTransportPolicy, GeminiByokFactory, ModelProviderSecretStore, GEMINI_ENDPOINT,
};
use mpdf_api_client::{Secret, SecretStore};
use mpdf_core::ocr_provider::credits;
use mpdf_core::ocr_provider::gemini::{CloudOcrConfig, DEFAULT_MODEL};
use mpdf_core::ocr_provider::structured_bbox;
use mpdf_core::ocr_provider::OcrProviderMode;

use crate::dto::{
    ConnectionTestDto, ConnectionTestRequestDto, CredentialSlotRequestDto, MaskedCredentialDto,
    OcrProviderModeDto, OcrProviderStatusDto, StoreCredentialRequestDto, UiErrorDto,
};
use crate::errors::request_error;

const DEFAULT_SLOT: &str = "default";

fn valid_slot(slot: &str) -> Result<&str, UiErrorDto> {
    if slot.is_empty()
        || slot.len() > 256
        || !slot
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(request_error(
            "invalid_parameter",
            "the credential slot name is not a plain identifier",
        ));
    }
    Ok(slot)
}

fn masked(slot: &str) -> MaskedCredentialDto {
    let described = ModelProviderSecretStore.describe(slot);
    MaskedCredentialDto {
        slot: described.slot,
        present: described.present,
        masked: described.masked,
    }
}

/// Everything the provider picker needs, computed from the core's own
/// capability data rather than restated in the UI.
#[tauri::command]
pub fn ocr_provider_status(slot: Option<String>) -> Result<OcrProviderStatusDto, UiErrorDto> {
    let slot = slot.unwrap_or_else(|| DEFAULT_SLOT.to_owned());
    let slot = valid_slot(&slot)?;
    let credential_present = ModelProviderSecretStore.describe(slot).present;
    let modes = OcrProviderMode::ALL
        .into_iter()
        .map(|mode| {
            let production_ready = mode.availability() == "stable";
            OcrProviderModeDto {
                id: mode.id().to_owned(),
                display_name: mode.display_name().to_owned(),
                default_mode: mode.is_default(),
                uses_network: mode.uses_network(),
                requires_credential: mode.uses_network(),
                execution_location: mode.execution_location().to_owned(),
                production_ready,
                availability: mode.availability().to_owned(),
                blockers: mode
                    .release_blockers()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                model: mode.uses_network().then(|| DEFAULT_MODEL.to_owned()),
                // Only the BYOK slot is a user-managed key; brokered mode uses
                // a session token this build cannot obtain.
                credential_present: matches!(mode, OcrProviderMode::GeminiByok)
                    && credential_present,
                structured_bbox_default_enabled: structured_bbox::GATES_VALIDATED_FOR_DEFAULT,
            }
        })
        .collect();
    Ok(OcrProviderStatusDto {
        default_mode: OcrProviderMode::Local.id().to_owned(),
        modes,
    })
}

/// Stores a key. The only command in this app that accepts one.
#[tauri::command]
pub fn store_model_provider_credential(
    request: StoreCredentialRequestDto,
) -> Result<MaskedCredentialDto, UiErrorDto> {
    let slot = valid_slot(&request.slot)?.to_owned();
    let secret = request.secret.trim().to_owned();
    if secret.is_empty() || secret.len() > 4096 {
        return Err(request_error(
            "invalid_parameter",
            "the key is empty or implausibly long",
        ));
    }
    // `request` owns the only other copy; dropping it here keeps the plaintext
    // out of the rest of this function's frame. `Secret` zeroizes on drop.
    drop(request);
    ModelProviderSecretStore
        .set(&slot, Secret::new(secret))
        .map_err(|error| {
            // The store's error, never the key's.
            request_error("input_error", error.to_string())
        })?;
    Ok(masked(&slot))
}

#[tauri::command]
pub fn model_provider_credential_status(
    request: CredentialSlotRequestDto,
) -> Result<MaskedCredentialDto, UiErrorDto> {
    let slot = valid_slot(&request.slot)?;
    Ok(masked(slot))
}

/// Removes the key. Explicit, reversible only by storing a new one, and the
/// documented way back to a fully local, offline configuration.
#[tauri::command]
pub fn delete_model_provider_credential(
    request: CredentialSlotRequestDto,
) -> Result<MaskedCredentialDto, UiErrorDto> {
    let slot = valid_slot(&request.slot)?;
    ModelProviderSecretStore
        .delete(slot)
        .map_err(|error| request_error("input_error", error.to_string()))?;
    Ok(masked(slot))
}

/// Non-billable connection test.
#[tauri::command]
pub fn test_ocr_provider(
    request: ConnectionTestRequestDto,
) -> Result<ConnectionTestDto, UiErrorDto> {
    let slot = valid_slot(&request.slot)?;
    let mode = OcrProviderMode::parse(&request.mode)
        .ok_or_else(|| request_error("invalid_parameter", "unknown provider mode"))?;
    match mode {
        OcrProviderMode::Local => Ok(ConnectionTestDto {
            mode: mode.id().to_owned(),
            provider_name: "local".into(),
            model: "tesseract".into(),
            model_available: true,
            credential: MaskedCredentialDto {
                slot: slot.to_owned(),
                present: false,
                masked: mpdf_core::ocr_provider::redaction::MASKED_CREDENTIAL.to_owned(),
            },
            diagnostic: "local OCR uses no network and needs no credential".into(),
        }),
        OcrProviderMode::GeminiByok => {
            if let Some(endpoint) = request.endpoint.as_deref() {
                if !endpoint.starts_with("https://") {
                    return Err(request_error(
                        "invalid_parameter",
                        "the provider endpoint must be HTTPS",
                    ));
                }
            }
            let factory = GeminiByokFactory::new(
                CloudOcrConfig::gemini_byok(slot),
                request.endpoint.as_deref().unwrap_or(GEMINI_ENDPOINT),
                CloudTransportPolicy::default(),
                "connection-test",
            );
            let result = factory.connection_test();
            Ok(ConnectionTestDto {
                mode: result.mode.id().to_owned(),
                provider_name: result.provider_name,
                model: result.model,
                model_available: result.model_available,
                credential: MaskedCredentialDto {
                    slot: result.credential.slot,
                    present: result.credential.present,
                    masked: result.credential.masked,
                },
                diagnostic: result.diagnostic,
            })
        }
        OcrProviderMode::MpdfCredits => Err(request_error(
            "not_available",
            format!(
                "M PDF Cloud OCR has no production service in this build: {}",
                credits::release_blockers().join("; ")
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_offers_three_modes_and_defaults_to_local() {
        let status = ocr_provider_status(None).unwrap();
        assert_eq!(status.default_mode, "local");
        assert_eq!(status.modes.len(), 3);
        let local = status
            .modes
            .iter()
            .find(|mode| mode.id == "local")
            .expect("local is offered");
        assert!(local.default_mode);
        assert!(!local.uses_network);
        assert!(!local.requires_credential);
    }

    #[test]
    fn the_picker_never_claims_credits_is_ready_for_production() {
        let status = ocr_provider_status(None).unwrap();
        let credits = status
            .modes
            .iter()
            .find(|mode| mode.id == "mpdf-credits")
            .unwrap();
        assert!(!credits.production_ready);
        assert!(!credits.blockers.is_empty());
    }

    #[test]
    fn the_picker_labels_gemini_as_beta() {
        let status = ocr_provider_status(None).unwrap();
        let gemini = status
            .modes
            .iter()
            .find(|mode| mode.id == "gemini-byok")
            .unwrap();
        assert_eq!(gemini.availability, "beta");
        assert!(!gemini.production_ready);
        assert!(!gemini.blockers.is_empty());
    }

    #[test]
    fn a_slot_name_must_be_a_plain_identifier() {
        for slot in ["", "../etc", "a b", &"x".repeat(300)] {
            assert!(
                model_provider_credential_status(CredentialSlotRequestDto {
                    slot: slot.to_owned()
                })
                .is_err(),
                "{slot:?} must be refused"
            );
        }
    }

    #[test]
    fn an_empty_key_is_refused_before_it_reaches_the_credential_store() {
        assert!(store_model_provider_credential(StoreCredentialRequestDto {
            slot: "default".into(),
            secret: "   ".into(),
        })
        .is_err());
    }

    #[test]
    fn a_plain_http_endpoint_is_refused_for_a_connection_test() {
        let error = test_ocr_provider(ConnectionTestRequestDto {
            mode: "gemini-byok".into(),
            slot: "default".into(),
            endpoint: Some("http://example.com".into()),
        })
        .unwrap_err();
        assert_eq!(error.code, "invalid_parameter");
    }

    #[test]
    fn the_brokered_mode_refuses_a_connection_test_with_its_blockers() {
        let error = test_ocr_provider(ConnectionTestRequestDto {
            mode: "mpdf-credits".into(),
            slot: "default".into(),
            endpoint: None,
        })
        .unwrap_err();
        assert!(error.message.contains("payment") || error.message.contains("production"));
    }
}
