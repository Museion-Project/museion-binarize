//! The one conversion point from [`ProcessingSettingsDto`] to the real
//! [`ProcessingSettings`], mirroring `mpdf-cli`'s
//! `SettingsArgs::to_settings` validation. The frontend constrains its
//! controls to sane ranges, but every bound here is re-checked
//! server-side — a frontend control range is a convenience, never trusted
//! as the actual limit (see `docs/desktop.md`).

use mpdf_core::binarization::SauvolaParams;
use mpdf_core::cleanup::{CleanupSettings, DespeckleLevel};
use mpdf_core::settings::{
    BinarizationMethod, PreprocessingSettings, ProcessingSettings, SUPPORTED_DPI,
};

use crate::dto::ProcessingSettingsDto;

pub fn to_processing_settings(dto: &ProcessingSettingsDto) -> Result<ProcessingSettings, String> {
    if !SUPPORTED_DPI.contains(&dto.dpi) {
        return Err(format!(
            "unsupported dpi {}; expected one of {:?}",
            dto.dpi, SUPPORTED_DPI
        ));
    }
    if !(-1.0..=1.0).contains(&dto.contrast) {
        return Err(format!(
            "contrast must be between -1.0 and 1.0, got {}",
            dto.contrast
        ));
    }
    if dto.background_radius.is_some() && !dto.background_normalization {
        return Err("backgroundRadius requires backgroundNormalization".to_string());
    }
    // Mirrors the CLI's SettingsArgs::to_settings exactly: a stale
    // threshold/Sauvola field left over from switching methods in the UI
    // must be rejected, not silently ignored, so a malformed or
    // inconsistent request is never accepted here only to be quietly
    // dropped a few lines down.
    if dto.threshold.is_some() && dto.method != "manual" {
        return Err("threshold only applies to method \"manual\"".to_string());
    }
    if (dto.sauvola_k.is_some() || dto.sauvola_window_size.is_some()) && dto.method != "sauvola" {
        return Err("sauvolaK and sauvolaWindowSize only apply to method \"sauvola\"".to_string());
    }

    let defaults = SauvolaParams::default();
    let method = match dto.method.as_str() {
        "otsu" => BinarizationMethod::Otsu,
        "manual" => {
            let threshold = dto
                .threshold
                .ok_or_else(|| "method \"manual\" requires threshold".to_string())?;
            BinarizationMethod::Manual { threshold }
        }
        "sauvola" => {
            let window_size = dto
                .sauvola_window_size
                .unwrap_or_else(|| mpdf_core::binarization::sauvola_window_for_dpi(dto.dpi));
            if window_size == 0 || window_size % 2 == 0 {
                return Err(format!(
                    "sauvolaWindowSize must be a positive odd number, got {window_size}"
                ));
            }
            BinarizationMethod::Sauvola(SauvolaParams {
                window_size,
                k: dto.sauvola_k.unwrap_or(defaults.k),
                dynamic_range: defaults.dynamic_range,
            })
        }
        other => return Err(format!("unknown method \"{other}\"")),
    };

    let despeckle = match dto.despeckle.as_str() {
        "off" => DespeckleLevel::Off,
        "conservative" => DespeckleLevel::Conservative,
        "strong" => DespeckleLevel::Strong,
        other => return Err(format!("unknown despeckle level \"{other}\"")),
    };

    let preprocessing_defaults = PreprocessingSettings::default();
    Ok(ProcessingSettings {
        dpi: dto.dpi,
        method,
        contrast: dto.contrast,
        preprocessing: PreprocessingSettings {
            median_denoise: dto.median_denoise,
            background_normalization: dto.background_normalization,
            background_radius: dto
                .background_radius
                .unwrap_or(preprocessing_defaults.background_radius),
        },
        cleanup: CleanupSettings {
            despeckle,
            max_component_pixels: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> ProcessingSettingsDto {
        ProcessingSettingsDto {
            dpi: 400,
            method: "otsu".to_string(),
            threshold: None,
            sauvola_window_size: None,
            sauvola_k: None,
            contrast: 0.0,
            median_denoise: false,
            background_normalization: false,
            background_radius: None,
            despeckle: "off".to_string(),
        }
    }

    #[test]
    fn otsu_converts_cleanly() {
        let settings = to_processing_settings(&base()).unwrap();
        assert!(matches!(settings.method, BinarizationMethod::Otsu));
        assert!(settings.validate().is_ok());
    }

    /// Parity with the CLI's `SettingsArgs::to_settings`: a threshold left
    /// over from a previous "manual" selection must be rejected when the
    /// method is no longer manual, not silently ignored. This is exactly
    /// the state the frontend would send if it forgot to clear the field
    /// on a method switch — see `SettingsPanel.tsx`'s `updateMethod`.
    #[test]
    fn a_stale_threshold_is_rejected_when_the_method_is_not_manual() {
        let mut dto = base();
        dto.method = "otsu".to_string();
        dto.threshold = Some(128);
        assert!(to_processing_settings(&dto)
            .unwrap_err()
            .contains("threshold"));
    }

    /// Same parity guarantee for the Sauvola-only fields.
    #[test]
    fn stale_sauvola_fields_are_rejected_when_the_method_is_not_sauvola() {
        let mut dto = base();
        dto.method = "otsu".to_string();
        dto.sauvola_k = Some(0.3);
        assert!(to_processing_settings(&dto)
            .unwrap_err()
            .contains("sauvola"));

        let mut dto = base();
        dto.method = "manual".to_string();
        dto.threshold = Some(100);
        dto.sauvola_window_size = Some(15);
        assert!(to_processing_settings(&dto)
            .unwrap_err()
            .contains("sauvola"));
    }

    #[test]
    fn manual_requires_a_threshold() {
        let mut dto = base();
        dto.method = "manual".to_string();
        assert!(to_processing_settings(&dto)
            .unwrap_err()
            .contains("threshold"));
        dto.threshold = Some(128);
        let settings = to_processing_settings(&dto).unwrap();
        assert!(matches!(
            settings.method,
            BinarizationMethod::Manual { threshold: 128 }
        ));
    }

    #[test]
    fn sauvola_defaults_come_from_the_core() {
        let mut dto = base();
        dto.method = "sauvola".to_string();
        let settings = to_processing_settings(&dto).unwrap();
        match settings.method {
            BinarizationMethod::Sauvola(p) => {
                assert_eq!(
                    p.window_size,
                    mpdf_core::binarization::sauvola_window_for_dpi(400)
                );
                assert_eq!(p.k, SauvolaParams::default().k);
            }
            other => panic!("unexpected method: {other:?}"),
        }
    }

    #[test]
    fn even_sauvola_window_is_rejected() {
        let mut dto = base();
        dto.method = "sauvola".to_string();
        dto.sauvola_window_size = Some(16);
        assert!(to_processing_settings(&dto).unwrap_err().contains("odd"));
    }

    #[test]
    fn unsupported_dpi_is_rejected_even_if_the_frontend_sent_it() {
        let mut dto = base();
        dto.dpi = 150;
        assert!(to_processing_settings(&dto).unwrap_err().contains("dpi"));
    }

    #[test]
    fn out_of_range_contrast_is_rejected() {
        let mut dto = base();
        dto.contrast = 5.0;
        assert!(to_processing_settings(&dto)
            .unwrap_err()
            .contains("contrast"));
    }

    #[test]
    fn background_radius_requires_normalization() {
        let mut dto = base();
        dto.background_radius = Some(20);
        assert!(to_processing_settings(&dto)
            .unwrap_err()
            .contains("backgroundNormalization"));
    }

    #[test]
    fn unknown_method_is_rejected() {
        let mut dto = base();
        dto.method = "ai-enhance".to_string();
        assert!(to_processing_settings(&dto).is_err());
    }

    #[test]
    fn every_supported_dpi_is_accepted() {
        for dpi in SUPPORTED_DPI {
            let mut dto = base();
            dto.dpi = dpi;
            assert!(to_processing_settings(&dto).is_ok());
        }
    }
}
