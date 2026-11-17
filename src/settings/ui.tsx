
import { useEffect, useState, type ReactNode } from "react";
import { Glyph, Tile, type GlyphName } from "./icons";

export function Group({
  title,
  footer,
  plain,
  children,
}: {
  title?: string;
  footer?: string;
  /** Dividers run from the left edge: the rows have no icons. */
  plain?: boolean;
  children: ReactNode;
}) {
  return (
    <section className="group">
      {title && <h2 className="gtitle">{title}</h2>}
      <div className={`card ${plain ? "plain" : ""}`}>{children}</div>
      {footer && <p className="gfoot">{footer}</p>}
    </section>
  );
}

interface RowProps {
  icon?: { glyph: GlyphName; color: string };
  label: ReactNode;
  sub?: ReactNode;
  value?: ReactNode;
  /** A chevron: the row opens a page. */
  opens?: boolean;
  /** An arrow out: the row opens something outside this window. */
  external?: boolean;
  onClick?: () => void;
  className?: string;
  children?: ReactNode;
}

export function Row({ icon, label, sub, value, opens, external, onClick, className = "", children }: RowProps) {
  return (
    <div
      className={`row ${onClick ? "tap" : ""} ${icon ? "" : "noicon"} ${className}`}
      onClick={onClick}
      role={onClick ? "button" : undefined}
    >
      {icon && <Tile glyph={icon.glyph} color={icon.color} />}
      <span className="label">
        {label}
        {sub && <small>{sub}</small>}
      </span>
      {value !== undefined && <span className="val">{value}</span>}
      {children}
      {opens && (
        <span className="chev">
          <Glyph name="chevron" size={16} />
        </span>
      )}
      {external && (
        <span className="chev">
          <Glyph name="external" size={15} />
        </span>
      )}
    </div>
  );
}

export function Switch({
  on,
  disabled,
  onChange,
}: {
  on: boolean;
  disabled?: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <button
      className={`switch ${on ? "on" : ""}`}
      role="switch"
      aria-checked={on}
      disabled={disabled}
      onClick={(e) => {
        // The row around it may open a page.
        e.stopPropagation();
        onChange(!on);
      }}
    />
  );
}

export function ToggleRow({
  icon,
  label,
  sub,
  on,
  disabled,
  onChange,
}: {
  icon?: RowProps["icon"];
  label: string;
  sub?: string;
  on: boolean;
  disabled?: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <Row
      icon={icon}
      label={label}
      sub={sub}
      className={disabled ? "disabled" : ""}
      onClick={disabled ? undefined : () => onChange(!on)}
    >
      <Switch on={on} disabled={disabled} onChange={onChange} />
    </Row>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className="seg" role="radiogroup">
      {options.map((option) => (
        <button
          key={option.value}
          role="radio"
          aria-checked={option.value === value}
          className={option.value === value ? "on" : ""}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function RadioRow({ label, on, onClick }: { label: ReactNode; on: boolean; onClick: () => void }) {
  return (
    <Row label={label} onClick={onClick}>
      <span className="radio">{on && <Glyph name="check" />}</span>
    </Row>
  );
}

export function NumberInput({
  value,
  min,
  max,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
}) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);

  const commit = () => {
    const parsed = Math.round(Number(text));
    const next = Number.isFinite(parsed) && text.trim() !== "" ? Math.min(max, Math.max(min, parsed)) : value;
    setText(String(next));
    if (next !== value) onChange(next);
  };

  return (
    <input
      className="number"
      inputMode="numeric"
      value={text}
      onClick={(e) => e.stopPropagation()}
      onChange={(e) => {
        setText(e.target.value);
        const parsed = Number(e.target.value);
        if (e.target.value.trim() !== "" && Number.isInteger(parsed) && parsed >= min && parsed <= max) {
          onChange(parsed);
        }
      }}
      onBlur={commit}
      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
    />
  );
}

export function NumberRow({
  label,
  unit,
  value,
  min,
  max,
  onChange,
}: {
  label: string;
  unit: string;
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
}) {
  return (
    <Row label={label}>
      <span className="unit">
        <NumberInput value={value} min={min} max={max} onChange={onChange} />
        {unit}
      </span>
    </Row>
  );
}

/** A small action in a row, in the accent color. */
export function Mini({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button
      className="mini"
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
    >
      {children}
    </button>
  );
}
