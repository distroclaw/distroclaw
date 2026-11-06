import { ReactNode, useId } from "react";

interface Props {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
  /** Only the switch, named by `label`, for use inside a row. */
  inline?: boolean;
  /** Shown next to the label, e.g. a badge. */
  badge?: ReactNode;
}

/** An on/off switch with its label and optional help text. */
export function Toggle({ checked, onChange, label, description, disabled = false, inline = false, badge }: Props) {
  const id = useId();
  const helpId = `${id}-help`;
  if (inline) {
    return (
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        className="toggle"
        disabled={disabled}
        onClick={() => onChange(!checked)}
      >
        <span className="toggle-knob" />
      </button>
    );
  }
  return (
    <section className="field">
      <div className="label-row">
        <label htmlFor={id}>{label}</label>
        {badge}
        <button
          id={id}
          type="button"
          role="switch"
          aria-checked={checked}
          aria-describedby={description ? helpId : undefined}
          className="toggle"
          disabled={disabled}
          onClick={() => onChange(!checked)}
        >
          <span className="toggle-knob" />
        </button>
      </div>
      {description && (
        <p id={helpId} className="field-help">
          {description}
        </p>
      )}
    </section>
  );
}
