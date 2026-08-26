//! User-facing processing settings.
//!
//! These types are the stable surface the CLI and desktop UI build on top
//! of. Changing a variant or field here is a breaking change for both
//! front ends.

use crate::binarization::SauvolaParams;
use crate::cleanup::CleanupSettings;
use crate::error::{CoreError, Result};

/// Output resolutions Phase 1 exposes in the UI and CLI.
pub const SUPPORTED_DPI: [u16; 3] = [300, 400, 600];

/// Which binarization algorithm to run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinarizationMethod {
    Otsu,
    Sauvola(SauvolaParams),
    Manual { threshold: u8 },
}

impl From<BinarizationMethod> for crate::binarization::BinarizationMethod {
    fn from(m: BinarizationMethod) -> Self {
        match m {
            BinarizationMethod::Otsu => crate::binarization::BinarizationMethod::Otsu,
            BinarizationMethod::Sauvola(p) => crate::binarization::BinarizationMethod::Sauvola(p),
            BinarizationMethod::Manual { threshold } => {
                crate::binarization::BinarizationMethod::Manual { threshold }
            }
        }
    }
}

/// Preprocessing toggles applied before binarization. Every field
/// defaults to its most conservative (usually "off") setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreprocessingSettings {
    pub median_denoise: bool,
    pub background_normalization: bool,
    /// Box-filter radius (pixels) used for background normalization.
    pub background_radius: u32,
}

impl Default for PreprocessingSettings {
    fn default() -> Self {
        Self {
            median_denoise: false,
            background_normalization: false,
            background_radius: 31,
        }
    }
}

/// Top-level processing settings for a single conversion job.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessingSettings {
    pub dpi: u16,
    pub method: BinarizationMethod,
    /// Grayscale contrast adjustment in `[-1.0, 1.0]`, applied before
    /// preprocessing and binarization.
    pub contrast: f32,
    pub preprocessing: PreprocessingSettings,
    pub cleanup: CleanupSettings,
}

impl ProcessingSettings {
    /// Validates that every field is within the range the pipeline can
    /// safely operate on. Called before processing begins; the pipeline
    /// never silently clamps or reinterprets an out-of-range user value.
    pub fn validate(&self) -> Result<()> {
        if !SUPPORTED_DPI.contains(&self.dpi) {
            return Err(CoreError::InvalidParameter(format!(
                "unsupported DPI {}; expected one of {:?}",
                self.dpi, SUPPORTED_DPI
            )));
        }
        if !(-1.0..=1.0).contains(&self.contrast) {
            return Err(CoreError::InvalidParameter(format!(
                "contrast must be in [-1.0, 1.0], got {}",
                self.contrast
            )));
        }
        if let BinarizationMethod::Sauvola(params) = self.method {
            if params.window_size == 0 || params.window_size % 2 == 0 {
                return Err(CoreError::InvalidParameter(format!(
                    "Sauvola window_size must be a positive odd number, got {}",
                    params.window_size
                )));
            }
            if params.dynamic_range <= 0.0 {
                return Err(CoreError::InvalidParameter(format!(
                    "Sauvola dynamic_range must be positive, got {}",
                    params.dynamic_range
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::{CleanupSettings, DespeckleLevel};

    fn base_settings() -> ProcessingSettings {
        ProcessingSettings {
            dpi: 400,
            method: BinarizationMethod::Otsu,
            contrast: 0.0,
            preprocessing: PreprocessingSettings::default(),
            cleanup: CleanupSettings {
                despeckle: DespeckleLevel::Off,
                max_component_pixels: None,
            },
        }
    }

    #[test]
    fn valid_settings_pass() {
        assert!(base_settings().validate().is_ok());
    }

    #[test]
    fn unsupported_dpi_is_rejected() {
        let mut s = base_settings();
        s.dpi = 150;
        assert!(s.validate().is_err());
    }

    #[test]
    fn out_of_range_contrast_is_rejected() {
        let mut s = base_settings();
        s.contrast = 2.0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn even_sauvola_window_is_rejected() {
        let mut s = base_settings();
        s.method = BinarizationMethod::Sauvola(SauvolaParams {
            window_size: 10,
            ..Default::default()
        });
        assert!(s.validate().is_err());
    }

    #[test]
    fn all_supported_dpis_are_accepted() {
        for dpi in SUPPORTED_DPI {
            let mut s = base_settings();
            s.dpi = dpi;
            assert!(s.validate().is_ok());
        }
    }
}
