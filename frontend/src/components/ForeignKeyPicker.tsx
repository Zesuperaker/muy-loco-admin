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
  const [active, setActive] = useState(-1);

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(query), 200);
    return () => clearTimeout(timer);
  }, [query]);

  const results = useQuery({
    queryKey: ["lookup", target, relation?.column, debounced],
    queryFn: () => api.lookup(target, debounced, relation?.column),
    enabled: open && Boolean(target),
  });

  const choices = results.data?.choices ?? [];
  const pick = (index: number) => {
    const choice = choices[index];
    if (!choice) return;
    onChange(keyText(choice.value), choice.label);
    setOpen(false);
  };
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
          aria-activedescendant={open && active >= 0 ? `${listId}-${active}` : undefined}
          className="input"
          placeholder={value ? (label ?? value) : placeholder}
          value={shown}
          disabled={disabled}
          onFocus={() => {
            setQuery("");
            setActive(-1);
            setOpen(true);
          }}
          onChange={(event) => {
            setQuery(event.target.value);
            setActive(-1);
            setOpen(true);
          }}
          onBlur={() => setTimeout(() => setOpen(false), 150)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setOpen(false);
            } else if (event.key === "ArrowDown") {
              event.preventDefault();
              setOpen(true);
              setActive((i) => Math.min(i + 1, choices.length - 1));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setActive((i) => Math.max(i - 1, 0));
            } else if (event.key === "Enter" && open && active >= 0) {
              event.preventDefault();
              pick(active);
            }
          }}
        />
        {value && !disabled && field.nullable && (
          <Button variant="ghost" onClick={() => onChange("", "")} aria-label={`Clear ${field.label}`}>
            Clear
          </Button>
        )}
      </div>
      {open && (
        <div
          id={listId}
          role="listbox"
          className="absolute z-20 mt-1 max-h-64 w-full overflow-auto rounded-md border border-slate-200 bg-white py-1 text-sm shadow-lg"
        >
          {results.isPending && <div className="px-3 py-2 text-slate-500">Searching…</div>}
          {results.error && <div className="px-3 py-2 text-rose-600">{results.error.message}</div>}
          {results.data && choices.length === 0 && (
            <div className="px-3 py-2 text-slate-500">No matches</div>
          )}
          {choices.map((choice, index) => {
            const text = keyText(choice.value);
            return (
              <div
                key={choice.key}
                id={`${listId}-${index}`}
                role="option"
                tabIndex={-1}
                aria-selected={text === value}
                className={`cursor-pointer px-3 py-2 hover:bg-indigo-50 ${
                  index === active ? "bg-indigo-50" : ""
                }`}
                onMouseDown={(event) => {
                  event.preventDefault();
                  pick(index);
                }}
              >
                {choice.label} <span className="text-xs text-slate-400">#{text}</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
