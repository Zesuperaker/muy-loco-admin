import { Combobox, ComboboxInput, ComboboxOption, ComboboxOptions } from "@headlessui/react";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api } from "../api";
import { keyText } from "../format";
import type { FieldMeta } from "../types";
import { Button } from "./ui";

interface Selection {
  value: string;
  label: string;
}

const status = "px-3 py-2 text-slate-500";

/** A searchable picker for a foreign-key field. `value` is the key as text. */
export function ForeignKeyPicker({
  field,
  value,
  label,
  onChange,
  disabled = false,
  id,
  placeholder = "Search…",
}: {
  field: FieldMeta;
  value: string;
  label?: string;
  onChange: (value: string, label: string) => void;
  disabled?: boolean;
  id?: string;
  placeholder?: string;
}) {
  const relation = field.relation;
  const target = relation?.model ?? "";
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");
  const [used, setUsed] = useState(false);

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query), 200);
    return () => clearTimeout(timer);
  }, [query]);

  const results = useQuery({
    queryKey: ["lookup", target, relation?.column, debounced],
    queryFn: () => api.lookup(target, debounced, relation?.column),
    enabled: used && Boolean(target),
  });
  const choices = results.data?.choices ?? [];
  const selected: Selection | null = value ? { value, label: label ?? value } : null;

  return (
    <div className="flex gap-2">
      <Combobox
        value={selected}
        by="value"
        immediate
        disabled={disabled}
        onChange={(choice: Selection | null) => {
          if (choice) onChange(choice.value, choice.label);
        }}
        onClose={() => setQuery("")}
      >
        <div className="relative flex-1">
          <ComboboxInput
            id={id}
            className="input"
            placeholder={placeholder}
            displayValue={(choice: Selection | null) => choice?.label ?? ""}
            onChange={(event) => setQuery(event.target.value)}
            onFocus={() => setUsed(true)}
          />
          <ComboboxOptions className="absolute z-20 mt-1 max-h-64 w-full overflow-auto rounded-md border border-slate-200 bg-white py-1 text-sm shadow-lg empty:invisible dark:border-slate-700 dark:bg-slate-800">
            {results.isPending && <div className={status}>Searching…</div>}
            {results.error && (
              <div className="px-3 py-2 text-rose-600">{results.error.message}</div>
            )}
            {results.data && choices.length === 0 && <div className={status}>No matches</div>}
            {choices.map((choice) => {
              const option = { value: keyText(choice.value), label: choice.label };
              return (
                <ComboboxOption
                  key={choice.key}
                  value={option}
                  className="cursor-pointer px-3 py-2 data-focus:bg-indigo-50 data-selected:font-medium dark:data-focus:bg-slate-700"
                >
                  {choice.label} <span className="text-xs text-slate-400">#{option.value}</span>
                </ComboboxOption>
              );
            })}
          </ComboboxOptions>
        </div>
      </Combobox>
      {value && !disabled && field.nullable && (
        <Button
          variant="ghost"
          onClick={() => onChange("", "")}
          aria-label={`Clear ${field.label}`}
        >
          Clear
        </Button>
      )}
    </div>
  );
}
