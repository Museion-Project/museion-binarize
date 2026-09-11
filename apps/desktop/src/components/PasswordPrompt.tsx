import { t, localizeMessage } from "../lib/i18n";
import { useState } from "react";

interface PasswordPromptProps {
  fileName: string;
  attemptError: string | null;
  onSubmit: (password: string) => void;
  onCancel: () => void;
}

/** Prompts for a document password. The password is held only in this
 * component's own local state, sent once on submit, and never persisted —
 * no "remember password" option exists. See docs/desktop.md, "Password-
 * protected PDFs". */
export function PasswordPrompt({
  fileName,
  attemptError,
  onSubmit,
  onCancel,
}: PasswordPromptProps) {
  const [password, setPassword] = useState("");

  function submit(e: React.FormEvent) {
    e.preventDefault();
    onSubmit(password);
    setPassword("");
  }

  return (
    <div className="modal-overlay" role="dialog" aria-modal="true" aria-label={t("输入 PDF 密码")}>
      <form className="password-prompt" onSubmit={submit}>
        <p>
          <strong>{fileName}</strong> {t("需要密码才能打开。")}</p>
        <label htmlFor="password-input">{t("密码")}</label>
        <input
          id="password-input"
          type="password"
          autoFocus
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        {attemptError && (
          <p className="password-prompt-error" role="alert">
            {localizeMessage(attemptError)}
          </p>
        )}
        <div className="password-prompt-actions">
          <button type="button" onClick={onCancel}>
            {t("取消")}</button>
          <button type="submit" className="primary" disabled={password.length === 0}>
            {t("打开")}</button>
        </div>
      </form>
    </div>
  );
}
