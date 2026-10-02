import { t } from "../lib/i18n";
import { Component, type ErrorInfo, type ReactNode } from "react";

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  error: Error | null;
}

/** A top-level boundary so a rendering bug shows a message instead of a
 * blank window. Does not replace structured backend errors (see
 * ErrorPanel) — this only catches mistakes in the React tree itself. */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    if (import.meta.env.DEV) {
      console.error("Unhandled error in the interface:", error, info.componentStack);
    }
  }

  render() {
    if (this.state.error) {
      return (
        <div className="error-boundary" role="alert">
          <h1>{t("界面发生错误。")}</h1>
          <p>{t("请重新启动应用。")}</p>
          {import.meta.env.DEV && (
            <pre className="error-boundary-detail">{String(this.state.error.stack ?? this.state.error)}</pre>
          )}
        </div>
      );
    }
    return this.props.children;
  }
}
