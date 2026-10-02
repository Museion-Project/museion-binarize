//! Provider selection and retained legacy credential command names.
//!
//! BYOK is disabled in this version. The old IPC commands return one stable
//! migration error without inspecting their payload or consulting a credential
//! store, so an older renderer cannot accidentally reactivate the path.

use mpdf_core::ocr_provider::credits;
use mpdf_core::ocr_provider::structured_bbox;
use mpdf_core::ocr_provider::{OcrProviderMode, BYOK_DISABLED_MESSAGE};

use crate::dto::{
    ConnectionTestDto, ConnectionTestRequestDto, CredentialSlotRequestDto, MaskedCredentialDto,
    OcrProviderModeDto, OcrProviderStatusDto, StoreCredentialRequestDto, UiErrorDto,
};
use crate::errors::request_error;

fn byok_disabled() -> UiErrorDto {
    request_error("byok_disabled", BYOK_DISABLED_MESSAGE)
}

/// Everything the provider picker needs, computed from the core's own
/// capability data rather than restated in the UI.
#[tauri::command]
pub fn ocr_provider_status(slot: Option<String>) -> Result<OcrProviderStatusDto, UiErrorDto> {
    let _ = slot;
    let modes = OcrProviderMode::PRODUCT_MODES
        .into_iter()
        .map(|mode| {
            let production_ready = mode.availability() == "stable";
            OcrProviderModeDto {
                id: mode.id().to_owned(),
                display_name: mode.display_name().to_owned(),
                default_mode: mode.is_default(),
                uses_network: mode.uses_network(),
                requires_credential: false,
                execution_location: mode.execution_location().to_owned(),
                production_ready,
                availability: mode.availability().to_owned(),
                blockers: mode
                    .release_blockers()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                model: None,
                credential_present: false,
                structured_bbox_default_enabled: structured_bbox::GATES_VALIDATED_FOR_DEFAULT,
            }
        })
        .collect();
    Ok(OcrProviderStatusDto {
        default_mode: OcrProviderMode::Local.id().to_owned(),
        modes,
    })
}

/// Rejects the retained legacy credential command without reading its payload.
#[tauri::command]
pub fn store_model_provider_credential(
    request: StoreCredentialRequestDto,
) -> Result<MaskedCredentialDto, UiErrorDto> {
    let _ = request;
    Err(byok_disabled())
}

#[tauri::command]
pub fn model_provider_credential_status(
    request: CredentialSlotRequestDto,
) -> Result<MaskedCredentialDto, UiErrorDto> {
    let _ = request;
    Err(byok_disabled())
}

/// Rejects the retained legacy credential deletion command without touching storage.
#[tauri::command]
pub fn delete_model_provider_credential(
    request: CredentialSlotRequestDto,
) -> Result<MaskedCredentialDto, UiErrorDto> {
    let _ = request;
    Err(byok_disabled())
}

/// Validates the selected product mode without making a network request.
#[tauri::command]
pub fn test_ocr_provider(
    request: ConnectionTestRequestDto,
) -> Result<ConnectionTestDto, UiErrorDto> {
    let mode = OcrProviderMode::parse(&request.mode)
        .ok_or_else(|| request_error("invalid_parameter", "unknown provider mode"))?;
    match mode {
        OcrProviderMode::Local => Ok(ConnectionTestDto {
            mode: mode.id().to_owned(),
            provider_name: "optional-local-ocr-plugin".into(),
            model: "not-inspected".into(),
            model_available: false,
            credential: MaskedCredentialDto {
                slot: "disabled".to_owned(),
                present: false,
                masked: mpdf_core::ocr_provider::redaction::MASKED_CREDENTIAL.to_owned(),
            },
            diagnostic: "the base app uses reliable native PDF text without a network; scanned pages require the separately installed and verified offline OCR plugin".into(),
        }),
        OcrProviderMode::GeminiByok => Err(byok_disabled()),
        OcrProviderMode::BrokerTest => Err(request_error("test_only", "broker-test is not offered in the product picker; use the explicitly configured debug pipeline")),
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
    fn the_picker_offers_current_modes_only_and_defaults_to_local() {
        let status = ocr_provider_status(None).unwrap();
        assert_eq!(status.default_mode, "local");
        assert_eq!(status.modes.len(), 2);
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
    fn the_picker_does_not_expose_legacy_byok() {
        let status = ocr_provider_status(None).unwrap();
        assert!(status.modes.iter().all(|mode| mode.id != "gemini-byok"));
    }

    #[test]
    fn local_connection_test_does_not_claim_the_optional_plugin_is_installed() {
        let result = test_ocr_provider(ConnectionTestRequestDto {
            mode: "local".into(),
            slot: "ignored".into(),
            endpoint: None,
        })
        .unwrap();
        assert!(!result.model_available);
        assert!(result.diagnostic.contains("native PDF text"));
        assert!(result.diagnostic.contains("offline OCR plugin"));
    }

    #[test]
    fn legacy_byok_commands_return_the_stable_disabled_error() {
        let error = test_ocr_provider(ConnectionTestRequestDto {
            mode: "gemini-byok".into(),
            slot: "../must-not-be-read".into(),
            endpoint: Some("http://must-not-be-read.invalid".into()),
        })
        .unwrap_err();
        assert_eq!(error.code, "byok_disabled");
        assert_eq!(error.message, BYOK_DISABLED_MESSAGE);

        let error = store_model_provider_credential(StoreCredentialRequestDto {
            slot: "ignored".into(),
            secret: "ignored-canary".into(),
        })
        .unwrap_err();
        assert_eq!(error.code, "byok_disabled");
        for command in [
            model_provider_credential_status
                as fn(CredentialSlotRequestDto) -> Result<MaskedCredentialDto, UiErrorDto>,
            delete_model_provider_credential,
        ] {
            let error = command(CredentialSlotRequestDto {
                slot: "../ignored".into(),
            })
            .unwrap_err();
            assert_eq!(error.code, "byok_disabled");
            assert_eq!(error.message, BYOK_DISABLED_MESSAGE);
        }
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
