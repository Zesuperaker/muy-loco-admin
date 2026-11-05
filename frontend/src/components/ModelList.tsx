import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { type ListQuery, api, listSearch } from "../api";
import { display, keyText } from "../format";
import { paths } from "../paths";
import type { AdminRecord, FieldMeta, Labels, ModelMeta } from "../types";
import { ForeignKeyPicker } from "./ForeignKeyPicker";
import { Button, Card, ErrorBanner, Spinner, useToast } from "./ui";

const NULL_FILTER = "__null__";

export function queryFromSearch(search: URLSearchParams): ListQuery {
  const filters: Record<string, string> = {};
  for (const [key, value] of search) {
    if (key.startsWith("filter.")) filters[key.slice("filter.".length)] = value;
  }
  const order = search.get("order");
  return {
    page: Math.max(1, Number(search.get("page")) || 1),
    sort: search.get("sort") ?? undefined,
    order: order === "asc" || order === "desc" ? order : undefined,
    q: search.get("q") ?? undefined,
    filters,
  };
}

function download(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  document.body.append(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}

function FilterControl({
  field,
  value,
  label,
  onChange,
}: {
  field: FieldMeta;
  value: string;
  label?: string;
  onChange: (value: string) => void;
}) {
  const id = `filter-${field.name}`;
  let control;
  if (field.relation?.model) {
    control = (
      <ForeignKeyPicker
        id={id}
        field={{ ...field, nullable: true }}
        value={value}
        label={label}
        placeholder="All"
        onChange={(next) => onChange(next)}
      />
    );
  } else if (field.kind === "boolean" || (field.choices?.length ?? 0) > 0) {
    const options =
      field.kind === "boolean"
        ? [
            ["true", "Yes"],
            ["false", "No"],
          ]
        : (field.choices ?? []).map((c) => [c, c]);
    control = (
      <select id={id} className="input" value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">All</option>
        {options.map(([v, text]) => (
          <option key={v} value={v}>
            {text}
          </option>
        ))}
        {field.nullable && <option value={NULL_FILTER}>Empty</option>}
      </select>
    );
  } else {
    control = (
      <input
        id={id}
        className="input"
        defaultValue={value}
        placeholder="Any"
        onKeyDown={(e) => {
          if (e.key === "Enter") onChange(e.currentTarget.value.trim());
        }}
        onBlur={(e) => onChange(e.currentTarget.value.trim())}
      />
    );
  }
  return (
    <div className="flex w-48 flex-col gap-1 text-xs font-medium text-slate-500">
      <label htmlFor={id}>{field.label}</label>
      {control}
    </div>
  );
}

function Cell({
  model,
  models,
  field,
  record,
  labels,
  first,
}: {
  model: ModelMeta;
  models: ModelMeta[];
  field: FieldMeta;
  record: AdminRecord;
  labels: Labels;
  first: boolean;
}) {
  const value = record.values[field.name];
  const text = display(field, value);
  if (first) {
    return (
      <Link to={paths.record(model.name, record.key)} className="font-medium text-indigo-600 hover:underline dark:text-indigo-400">
        {text}
      </Link>
    );
  }
  const relation = field.relation;
  if (relation?.model && value !== null && value !== undefined) {
    const label = labels[field.name]?.[keyText(value)] ?? text;
    const target = models.find((m) => m.name === relation.model);
    const pk = target?.primary_key;
    if (pk?.length === 1 && pk[0] === relation.column) {
      return (
        <Link to={paths.record(relation.model, keyText(value))} className="text-indigo-600 hover:underline dark:text-indigo-400">
          {label}
        </Link>
      );
    }
    return <span>{label}</span>;
  }
  if (field.kind === "boolean" && typeof value === "boolean") {
    return (
      <span
        className={`rounded-full px-2 py-0.5 text-xs font-medium ${
          value
            ? "bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300"
            : "bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300"
        }`}
      >
        {text}
      </span>
    );
  }
  return <span className={value === null ? "text-slate-400" : ""}>{text}</span>;
}

export function ModelList({ model, models }: { model: ModelMeta; models: ModelMeta[] }) {
  const [search, setSearch] = useSearchParams();
  const query = queryFromSearch(search);
  const scope = `${model.name}?${search.toString()}`;
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const [selection, setSelection] = useState<{ scope: string; keys: Set<string> }>({
    scope,
    keys: new Set(),
  });
  const selected = selection.scope === scope ? selection.keys : new Set<string>();
  const [q, setQ] = useState(query.q ?? "");

  const list = useQuery({
    queryKey: ["list", model.name, search.toString()],
    queryFn: () => api.list(model.name, query),
    placeholderData: keepPreviousData,
  });

  const update = (changes: Partial<ListQuery>) => {
    const next = listSearch({ ...query, page: 1, ...changes });
    setSearch(new URLSearchParams(next.slice(1)));
  };

  const refresh = () => {
    setSelection({ scope, keys: new Set() });
    void queryClient.invalidateQueries({ queryKey: ["list", model.name] });
    void queryClient.invalidateQueries({ queryKey: ["stats"] });
  };

  const bulkDelete = useMutation({
    mutationFn: (keys: string[]) => api.bulkDelete(model.name, keys),
    onSuccess: (res) => {
      notify(`Deleted ${res.deleted} ${res.deleted === 1 ? "record" : "records"}`);
      refresh();
    },
    onError: (err) => notify(err.message, "error"),
  });

  const runAction = useMutation({
    mutationFn: ({ action, keys }: { action: string; keys: string[] }) =>
      api.action(model.name, action, keys),
    onSuccess: (res) => {
      notify(res.message || "Done");
      refresh();
    },
    onError: (err) => notify(err.message, "error"),
  });

  const exportCsv = useMutation({
    mutationFn: () => api.exportCsv(model.name, query),
    onSuccess: (blob) => download(blob, `${model.name}.csv`),
    onError: (err) => notify(err.message, "error"),
  });

  const columns = model.list_display
    .map((name) => model.fields.find((f) => f.name === name))
    .filter((f): f is FieldMeta => Boolean(f));
  const filters = model.list_filter
    .map((name) => model.fields.find((f) => f.name === name))
    .filter((f): f is FieldMeta => Boolean(f));
  const sortField = query.sort ?? model.ordering.field;
  const sortOrder = query.order ?? (query.sort ? "asc" : model.ordering.direction);
  const selectable = model.capabilities.delete || model.actions.length > 0;
  const page = list.data;
  const records = page?.records ?? [];
  const allSelected = records.length > 0 && records.every((r) => selected.has(r.key));

  const toggle = (key: string) => {
    const keys = new Set(selected);
    if (keys.has(key)) keys.delete(key);
    else keys.add(key);
    setSelection({ scope, keys });
  };
  const toggleAll = () => {
    setSelection({ scope, keys: allSelected ? new Set() : new Set(records.map((r) => r.key)) });
  };
  const sortBy = (name: string) => {
    const order = sortField === name && sortOrder === "asc" ? "desc" : "asc";
    update({ sort: name, order });
  };
  const submitSearch = (event: FormEvent) => {
    event.preventDefault();
    update({ q: q.trim() || undefined });
  };

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold">{model.label}</h1>
          {page && <p className="text-sm text-slate-500">{page.total} total</p>}
        </div>
        <div className="flex gap-2">
          {model.capabilities.export && (
            <Button onClick={() => exportCsv.mutate()} disabled={exportCsv.isPending}>
              Export CSV
            </Button>
          )}
          {model.capabilities.create && (
            <Link
              to={paths.create(model.name)}
              className="inline-flex items-center rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white shadow-sm hover:bg-indigo-500"
            >
              Add record
            </Link>
          )}
        </div>
      </div>

      {(model.search_fields.length > 0 || filters.length > 0) && (
        <Card className="flex flex-wrap items-end gap-4 p-4">
          {model.search_fields.length > 0 && (
            <form role="search" onSubmit={submitSearch} className="flex flex-1 items-end gap-2">
              <div className="flex min-w-48 flex-1 flex-col gap-1 text-xs font-medium text-slate-500">
                <label htmlFor="search">Search</label>
                <input
                  id="search"
                  type="search"
                  className="input"
                  placeholder={`Search ${model.search_fields.join(", ")}`}
                  value={q}
                  onChange={(e) => setQ(e.target.value)}
                />
              </div>
              <Button type="submit">Search</Button>
            </form>
          )}
          {filters.map((field) => (
            <FilterControl
              key={`${field.name}:${query.filters?.[field.name] ?? ""}`}
              field={field}
              value={query.filters?.[field.name] ?? ""}
              label={page?.labels[field.name]?.[query.filters?.[field.name] ?? ""]}
              onChange={(value) => update({ filters: { ...query.filters, [field.name]: value } })}
            />
          ))}
          {(query.q || Object.values(query.filters ?? {}).some(Boolean)) && (
            <Button
              variant="ghost"
              onClick={() => {
                setQ("");
                update({ q: undefined, filters: {} });
              }}
            >
              Clear
            </Button>
          )}
        </Card>
      )}

      {selected.size > 0 && (
        <div className="flex flex-wrap items-center gap-2 rounded-md bg-indigo-50 px-4 py-2 text-sm dark:bg-indigo-950">
          <span className="font-medium">{selected.size} selected</span>
          {model.actions.map((action) => (
            <Button
              key={action.name}
              disabled={runAction.isPending}
              onClick={() => runAction.mutate({ action: action.name, keys: [...selected] })}
            >
              {action.label}
            </Button>
          ))}
          {model.capabilities.delete && (
            <Button
              variant="danger"
              disabled={bulkDelete.isPending}
              onClick={() => {
                if (window.confirm(`Delete ${selected.size} selected records?`)) {
                  bulkDelete.mutate([...selected]);
                }
              }}
            >
              Delete selected
            </Button>
          )}
        </div>
      )}

      <ErrorBanner error={list.error} />
      {list.isPending ? (
        <Spinner />
      ) : (
        <Card className="overflow-x-auto">
          <table className="min-w-full divide-y divide-slate-200 text-sm dark:divide-slate-800">
            <thead className="bg-slate-50 text-left text-xs font-semibold tracking-wide text-slate-500 uppercase dark:bg-slate-800/50">
              <tr>
                {selectable && (
                  <th className="w-10 px-4 py-2">
                    <input type="checkbox" aria-label="Select all" checked={allSelected} onChange={toggleAll} />
                  </th>
                )}
                {columns.map((field) => (
                  <th
                    key={field.name}
                    className="px-4 py-2"
                    aria-sort={
                      sortField === field.name
                        ? sortOrder === "asc"
                          ? "ascending"
                          : "descending"
                        : undefined
                    }
                  >
                    <button type="button" className="uppercase hover:text-slate-900 dark:hover:text-white" onClick={() => sortBy(field.name)}>
                      {field.label}
                      {sortField === field.name && (sortOrder === "asc" ? " ▲" : " ▼")}
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
              {records.length === 0 && (
                <tr>
                  <td colSpan={columns.length + 1} className="px-4 py-8 text-center text-slate-500">
                    No records found.
                  </td>
                </tr>
              )}
              {records.map((record) => (
                <tr key={record.key} className="hover:bg-slate-50 dark:hover:bg-slate-800/40">
                  {selectable && (
                    <td className="px-4 py-2">
                      <input
                        type="checkbox"
                        aria-label={`Select ${record.label}`}
                        checked={selected.has(record.key)}
                        onChange={() => toggle(record.key)}
                      />
                    </td>
                  )}
                  {columns.map((field, index) => (
                    <td key={field.name} className="px-4 py-2 whitespace-nowrap">
                      <Cell
                        model={model}
                        models={models}
                        field={field}
                        record={record}
                        labels={page?.labels ?? {}}
                        first={index === 0}
                      />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}

      {page && page.pages > 1 && (
        <nav aria-label="Pagination" className="flex items-center justify-between text-sm">
          <span className="text-slate-500">
            Page {page.page} of {page.pages}
          </span>
          <div className="flex gap-2">
            <Button disabled={page.page <= 1} onClick={() => update({ page: page.page - 1 })}>
              Previous
            </Button>
            <Button disabled={page.page >= page.pages} onClick={() => update({ page: page.page + 1 })}>
              Next
            </Button>
          </div>
        </nav>
      )}
    </div>
  );
}
