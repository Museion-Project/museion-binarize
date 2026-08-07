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
    <div className="modal-overlay" role="dialog" aria-modal="true" aria-label="Password required">
      <form className="password-prompt" onSubmit={submit}>
        <p>
          <strong>{fileName}</strong> is password-protected.
        </p>
        <label htmlFor="password-input">Password</label>
        <input
          id="password-input"
          type="password"
          autoFocus
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        {attemptError && (
          <p className="password-prompt-error" role="alert">
            {attemptError}
          </p>
        )}
        <div className="password-prompt-actions">
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={password.length === 0}>
            Open
          </button>
        </div>
      </form>
    </div>
  );
}
