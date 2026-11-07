import type { FieldMeta } from "../types";
import { ForeignKeyPicker } from "./ForeignKeyPicker";

export function FieldInput({
  field,
  value,
  onChange,
  disabled,
  fkLabel,
  onPick,
}: {
  field: FieldMeta;
  value: string;
  onChange: (value: string) => void;
  disabled: boolean;
  fkLabel?: string;
  onPick?: (value: string, label: string) => void;
}) {
  const id = `field-${field.name}`;
  const common = { id, disabled, className: "input", name: field.name };

  if (field.relation?.model && onPick) {
    return (
      <ForeignKeyPicker
        id={id}
        field={field}
        value={value}
        label={fkLabel}
        disabled={disabled}
        onChange={onPick}
      />
    );
  }

  const choices = field.choices ?? [];
  if (choices.length > 0) {
    return (
      <select {...common} value={value} onChange={(e) => onChange(e.target.value)}>
        {(field.nullable || value === "") && <option value="">—</option>}
        {choices.map((choice) => (
          <option key={choice} value={choice}>
            {choice}
          </option>
        ))}
      </select>
    );
  }

  switch (field.kind) {
    case "boolean":
      if (field.nullable) {
        return (
          <select {...common} value={value} onChange={(e) => onChange(e.target.value)}>
            <option value="">—</option>
            <option value="true">Yes</option>
            <option value="false">No</option>
          </select>
        );
      }
      return (
        <input
          id={id}
          name={field.name}
          type="checkbox"
          disabled={disabled}
          className="h-4 w-4 rounded border-slate-300 text-indigo-600"
          checked={value === "true"}
          onChange={(e) => onChange(e.target.checked ? "true" : "false")}
        />
      );
    case "text":
      return (
        <textarea {...common} rows={5} value={value} onChange={(e) => onChange(e.target.value)} />
      );
    case "json":
    case "array":
      return (
        <textarea
          {...common}
          rows={6}
          spellCheck={false}
          className="input font-mono text-xs"
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      );
    case "integer":
      // Text, not `type="number"`: number inputs hold a double, which rounds
      // integers beyond 2^53. `fromInput` checks the digits.
      return (
        <input {...common} inputMode="numeric" value={value} onChange={(e) => onChange(e.target.value)} />
      );
    case "float":
      return (
        <input {...common} type="number" step="any" value={value} onChange={(e) => onChange(e.target.value)} />
      );
    case "decimal":
      return (
        <input {...common} inputMode="decimal" value={value} onChange={(e) => onChange(e.target.value)} />
      );
    case "date":
      return <input {...common} type="date" value={value} onChange={(e) => onChange(e.target.value)} />;
    case "time":
      return (
        <input {...common} type="time" step={1} value={value} onChange={(e) => onChange(e.target.value)} />
      );
    case "date_time":
      return (
        <input
          {...common}
          type="datetime-local"
          step={1}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      );
    default:
      return (
        <input
          {...common}
          type="text"
          maxLength={field.max_length}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      );
  }
}
