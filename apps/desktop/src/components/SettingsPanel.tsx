import type { PresetId, ProcessingSettings } from "../app/types";
import { PRESETS, matchingPreset } from "../lib/settings";

interface SettingsPanelProps {
  settings: ProcessingSettings;
  preset: PresetId;
  disabled: boolean;
  onChange: (settings: ProcessingSettings, preset: PresetId) => void;
}

const DPI_OPTIONS = [300, 400, 600] as const;

export function SettingsPanel({ settings, preset, disabled, onChange }: SettingsPanelProps) {
  function update(partial: Partial<ProcessingSettings>) {
    const next = { ...settings, ...partial };
    onChange(next, matchingPreset(next));
  }

  return (
    <section className="settings-panel" aria-label="Processing settings">
      <fieldset disabled={disabled}>
        <legend>Presets</legend>
        <div className="preset-buttons" role="group" aria-label="Presets">
          {(Object.entries(PRESETS) as [PresetId, (typeof PRESETS)[keyof typeof PRESETS]][]).map(
            ([id, definition]) => (
              <button
                key={id}
                type="button"
                className={preset === id ? "selected" : ""}
                title={definition.description}
                onClick={() => onChange(definition.settings, id)}
              >
                {definition.label}
              </button>
            ),
          )}
          <span className={`preset-custom-badge${preset === "custom" ? " active" : ""}`}>
            {preset === "custom" ? "Custom" : ""}
          </span>
        </div>
      </fieldset>

      <fieldset disabled={disabled}>
        <legend>Method</legend>
        <label htmlFor="method-select">Binarization method</label>
        <select
          id="method-select"
          value={settings.method}
          onChange={(e) => update({ method: e.target.value as ProcessingSettings["method"] })}
        >
          <option value="otsu">Otsu</option>
          <option value="sauvola">Sauvola</option>
          <option value="manual">Manual</option>
        </select>

        {settings.method === "manual" && (
          <>
            <label htmlFor="threshold-input">Threshold (0–255)</label>
            <input
              id="threshold-input"
              type="number"
              min={0}
              max={255}
              value={settings.threshold ?? 128}
              onChange={(e) => update({ threshold: Number(e.target.value) })}
            />
          </>
        )}

        {settings.method === "sauvola" && (
          <>
            <label htmlFor="sauvola-window-input">Window size (odd)</label>
            <input
              id="sauvola-window-input"
              type="number"
              min={3}
              step={2}
              value={settings.sauvolaWindowSize ?? ""}
              placeholder="Derived from DPI"
              onChange={(e) =>
                update({
                  sauvolaWindowSize: e.target.value === "" ? null : Number(e.target.value),
                })
              }
            />
            <label htmlFor="sauvola-k-input">k (sensitivity)</label>
            <input
              id="sauvola-k-input"
              type="number"
              min={0.05}
              max={0.9}
              step={0.05}
              value={settings.sauvolaK ?? ""}
              placeholder="0.20"
              onChange={(e) =>
                update({ sauvolaK: e.target.value === "" ? null : Number(e.target.value) })
              }
            />
          </>
        )}
      </fieldset>

      <fieldset disabled={disabled}>
        <legend>Output</legend>
        <label htmlFor="dpi-select">Resolution (DPI)</label>
        <select
          id="dpi-select"
          value={settings.dpi}
          onChange={(e) => update({ dpi: Number(e.target.value) })}
        >
          {DPI_OPTIONS.map((dpi) => (
            <option key={dpi} value={dpi}>
              {dpi}
            </option>
          ))}
        </select>

        <label htmlFor="contrast-input">
          Contrast ({settings.contrast > 0 ? "+" : ""}
          {settings.contrast.toFixed(2)})
        </label>
        <input
          id="contrast-input"
          type="range"
          min={-1}
          max={1}
          step={0.05}
          value={settings.contrast}
          onChange={(e) => update({ contrast: Number(e.target.value) })}
        />
      </fieldset>

      <fieldset disabled={disabled}>
        <legend>Preprocessing</legend>
        <label>
          <input
            type="checkbox"
            checked={settings.medianDenoise}
            onChange={(e) => update({ medianDenoise: e.target.checked })}
          />
          Median denoise
        </label>
        <label>
          <input
            type="checkbox"
            checked={settings.backgroundNormalization}
            onChange={(e) =>
              update({
                backgroundNormalization: e.target.checked,
                backgroundRadius: e.target.checked ? settings.backgroundRadius : null,
              })
            }
          />
          Background normalization
        </label>
        {settings.backgroundNormalization && (
          <>
            <label htmlFor="background-radius-input">Background radius (px)</label>
            <input
              id="background-radius-input"
              type="number"
              min={1}
              value={settings.backgroundRadius ?? ""}
              placeholder="31"
              onChange={(e) =>
                update({
                  backgroundRadius: e.target.value === "" ? null : Number(e.target.value),
                })
              }
            />
          </>
        )}
      </fieldset>

      <fieldset disabled={disabled}>
        <legend>Cleanup</legend>
        <label htmlFor="despeckle-select">Despeckle</label>
        <select
          id="despeckle-select"
          value={settings.despeckle}
          onChange={(e) =>
            update({ despeckle: e.target.value as ProcessingSettings["despeckle"] })
          }
        >
          <option value="off">Off</option>
          <option value="conservative">Conservative</option>
          <option value="strong">Strong</option>
        </select>
      </fieldset>
    </section>
  );
}
