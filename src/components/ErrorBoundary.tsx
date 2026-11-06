import { Component, ErrorInfo, ReactNode } from "react";
import { errorText } from "../lib/api";
import { Logo } from "./Icons";

interface ErrorCardProps {
  title: string;
  message: string;
  actionLabel: string;
  onAction: () => void;
}

/** Full-window error with one way out, so the window is never blank. */
export function ErrorCard({ title, message, actionLabel, onAction }: ErrorCardProps) {
  return (
    <div className="center-screen">
      <div className="card engine-gate" role="alert">
        <Logo size={48} />
        <h2>{title}</h2>
        <p className="muted">{message}</p>
        <button className="btn primary" onClick={onAction}>
          {actionLabel}
        </button>
      </div>
    </div>
  );
}

interface State {
  error: string | null;
}

export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(e: unknown): State {
    return { error: errorText(e) };
  }

  componentDidCatch(e: unknown, info: ErrorInfo) {
    console.error(e, info.componentStack);
  }

  render() {
    if (this.state.error === null) return this.props.children;
    return (
      <ErrorCard
        title="Something went wrong"
        message={this.state.error}
        actionLabel="Reload"
        onAction={() => location.reload()}
      />
    );
  }
}
