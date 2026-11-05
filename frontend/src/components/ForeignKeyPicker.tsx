import { useQuery } from "@tanstack/react-query";
import { useEffect, useId, useState } from "react";
import { api } from "../api";
import { keyText } from "../format";
import type { FieldMeta } from "../types";
import { Button } from "./ui";

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
  const listId = useId();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query), 200);
    return () => clearTimeout(timer);
  }, [query]);

  const results = useQuery({
    queryKey: ["lookup", target, relation?.column, debounced],
    queryFn: () => api.lookup(target, debounced, relation?.column),
    enabled: open && Boolean(target),
  });

  const shown = open ? query : (label ?? value);
  return (
    <div className="relative">
      <div className="flex gap-2">
        <input
          id={id}
          role="combobox"
          aria-expanded={open}
          aria-controls={listId}
          aria-autocomplete="list"
          className="input"
          placeholder={value ? (label ?? value) : placeholder}
          value={shown}
          disabled={disabled}
          onFocus={() => {
            setQuery("");
            setOpen(true);
          }}
          onChange={(event) => setQuery(event.target.value)}
          onBlur={() => setTimeout(() => setOpen(false), 150)}
          onKeyDown={(event) => {
            if (event.key === "Escape") setOpen(false);
          }}
        />
        {value && !disabled && field.nullable && (
          <Button variant="ghost" onClick={() => onChange("", "")} aria-label={`Clear ${field.label}`}>
            Clear
          </Button>
        )}
      </div>
      {open && (
        <ul
          id={listId}
          role="listbox"
          className="absolute z-20 mt-1 max-h-64 w-full overflow-auto rounded-md border border-slate-200 bg-white py-1 text-sm shadow-lg dark:border-slate-700 dark:bg-slate-800"
        >
          {results.isPending && <li className="px-3 py-2 text-slate-500">Searching…</li>}
          {results.error && <li className="px-3 py-2 text-rose-600">{results.error.message}</li>}
          {results.data?.choices.length === 0 && (
            <li className="px-3 py-2 text-slate-500">No matches</li>
          )}
          {results.data?.choices.map((choice) => {
            const text = keyText(choice.value);
            return (
              <li
                key={choice.key}
                role="option"
                aria-selected={text === value}
                className="cursor-pointer px-3 py-2 hover:bg-indigo-50 dark:hover:bg-slate-700"
                onMouseDown={(event) => {
                  event.preventDefault();
                  onChange(text, choice.label);
                  setOpen(false);
                }}
              >
                {choice.label} <span className="text-xs text-slate-400">#{text}</span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
