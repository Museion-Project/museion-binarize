import { t, localizeMessage } from "../lib/i18n";
import { useState } from "react";

import type { UiError } from "../app/types";

interface ErrorPanelProps {
  error: UiError;
  onDismiss: () => void;
}

/** A structured, actionable error panel. Never interprets `error.message`
 * to decide anything — only `error.code` (via the caller) does that; this
 * component just displays what the backend already classified. See
 * docs/desktop.md, "Error UX". */
export function ErrorPanel({ error, onDismiss }: ErrorPanelProps) {
  const [showDetail, setShowDetail] = useState(false);

  return (
    <div className="error-panel" role="alert">
      <p className="error-panel-message">{localizeMessage(error.message)}</p>
      {error.hint && <p className="error-panel-hint">{localizeMessage(error.hint)}</p>}
      <div className="error-panel-actions">
        {error.detail && (
          <button type="button" onClick={() => setShowDetail((v) => !v)}>
            {showDetail ? t("收起详细信息") : t("查看详细信息")}
          </button>
        )}
        <button type="button" onClick={onDismiss}>
          {t("返回")}</button>
      </div>
      {showDetail && error.detail && <pre className="error-panel-detail">{error.detail}</pre>}
    </div>
  );
}
