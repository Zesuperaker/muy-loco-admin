import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, type MouseEvent, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { type ListQuery, api, listSearch } from "../api";
import { countNote, display, encodeKeyPart, formatCount, keyText } from "../format";
import { paths } from "../paths";
import type { AdminRecord, FieldMeta, Labels, ModelMeta } from "../types";
import { ForeignKeyPicker } from "./ForeignKeyPicker";
import { useCanManageRoles } from "../user";
import { RoleSelect } from "./RoleSelect";
import { Button, Card, ErrorBanner, Spinner, useToast } from "./ui";

const NULL_FILTER = "__null__";
/** Clicks on these keep their own behavior instead of opening the row's record. */
const INTERACTIVE = "a, button, input, select, textarea, [data-select]";

export function queryFromSearch(search: URLSearchParams): ListQuery {
  const filters: Record<string, string> = {};
  for (const [key, value] of search) {
    if (key.startsWith("filter.")) filters[key.slice("filter.".length)] = value;
  }
  const order = search.get("order");
  const after = search.get("after") || undefined;
  return {
    page: Math.max(1, Number(search.get("page")) || 1),
    after,
    before: after ? undefined : search.get("before") || undefined,
    sort: search.get("sort") ?? undefined,
    order: order === "asc" || order === "desc" ? order : undefined,
    q: search.get("q") ?? undefined,
    filters,
  };
}

const GAP = "…";

/** `from` to `to`, inclusive. */
const span = (from: number, to: number) =>
  Array.from({ length: Math.max(0, to - from + 1) }, (_, i) => from + i);

/**
 * The page numbers to offer, like Django's elided page range: every page of a short list, else
 * the first and last pages and those around `current`, with gaps between.
 */
export function pageRange(current: number, total: number, onEachSide = 2, onEnds = 1) {
  if (total <= (onEachSide + onEnds) * 2) return span(1, total);
  const page = Math.min(current, total);
  const out: (number | typeof GAP)[] = [];
  if (page > 1 + onEachSide + onEnds + 1) {
    out.push(...span(1, onEnds), GAP, ...span(page - onEachSide, page));
  } else {
    out.push(...span(1, page));
  }
  if (page < total - onEachSide - onEnds - 1) {
    out.push(...span(page + 1, page + onEachSide), GAP, ...span(total - onEnds + 1, total));
  } else {
    out.push(...span(page + 1, total));
  }
  return out;
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
  const canManageRoles = useCanManageRoles();
  let control;
  if (field.role && canManageRoles) {
    control = (
      <RoleSelect id={id} value={value} emptyLabel="All" onChange={onChange}>
        <option value={NULL_FILTER}>No role</option>
      </RoleSelect>
    );
  } else if (field.relation?.model) {
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
    // Only a changed value is applied, so leaving the field keeps the page.
    const commit = (text: string) => {
      if (text.trim() !== value) onChange(text.trim());
    };
    control = (
      <input
        id={id}
        className="input"
        defaultValue={value}
        placeholder="Any"
        onKeyDown={(e) => {
          if (e.key === "Enter") commit(e.currentTarget.value);
        }}
        onBlur={(e) => commit(e.currentTarget.value)}
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
  link,
}: {
  model: ModelMeta;
  models: ModelMeta[];
  field: FieldMeta;
  record: AdminRecord;
  labels: Labels;
  link: boolean;
}) {
  const value = record.values[field.name];
  const text = display(field, value);
  if (link) {
    return (
      <Link to={paths.record(model.name, record.key)} className="font-medium text-indigo-600 hover:underline">
        {text}
      </Link>
    );
  }
  if (field.role && value !== null && value !== undefined) {
    return <span>{labels[field.name]?.[keyText(value)] ?? text}</span>;
  }
  const relation = field.relation;
  if (relation?.model && value !== null && value !== undefined) {
    const label = labels[field.name]?.[keyText(value)] ?? text;
    const target = models.find((m) => m.name === relation.model);
    const pk = target?.primary_key;
    if (pk?.length === 1 && pk[0] === relation.column) {
      return (
        <Link
          to={paths.record(relation.model, encodeKeyPart(keyText(value)))}
          className="text-indigo-600 hover:underline"
        >
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
            ? "bg-emerald-100 text-emerald-800"
            : "bg-slate-100 text-slate-600"
        }`}
      >
        {text}
      </span>
    );
  }
  return <span className={value === null ? "text-slate-400" : ""}>{text}</span>;
}

function NumberedPages({
  page,
  pages,
  hasNext,
  onPage,
}: {
  page: number;
  /** The last page, once the count is in. */
  pages: number | undefined;
  hasNext: boolean;
  onPage: (page: number) => void;
}) {
  return (
    <nav aria-label="Pagination" className="flex flex-wrap items-center justify-between gap-2 text-sm">
      <span className="text-slate-500">
        Page {page}
        {pages !== undefined && ` of ${pages}`}
      </span>
      <div className="flex flex-wrap items-center gap-1">
        <Button disabled={page <= 1} onClick={() => onPage(page - 1)}>
          Previous
        </Button>
        {pages !== undefined &&
          pageRange(page, pages).map((n, i) =>
            n === GAP ? (
              <span key={`gap-${i}`} className="px-1 text-slate-400">
                {GAP}
              </span>
            ) : (
              <Button
                key={n}
                variant={n === page ? "primary" : "secondary"}
                aria-current={n === page ? "page" : undefined}
                aria-label={`Page ${n}`}
                onClick={() => onPage(n)}
              >
                {n}
              </Button>
            ),
          )}
        <Button disabled={!hasNext} onClick={() => onPage(page + 1)}>
          Next
        </Button>
      </div>
    </nav>
  );
}

export function ModelList({ model, models }: { model: ModelMeta; models: ModelMeta[] }) {
  const [search, setSearch] = useSearchParams();
  const query = queryFromSearch(search);
  const scope = `${model.name}?${search.toString()}`;
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const { notify } = useToast();
  const [selection, setSelection] = useState<{ scope: string; keys: Set<string> }>({
    scope,
    keys: new Set(),
  });
  const selected = selection.scope === scope ? selection.keys : new Set<string>();
  const [q, setQ] = useState(query.q ?? "");
  // Follow the URL when it changes another way (Back, the sidebar).
  const [urlQ, setUrlQ] = useState(query.q);
  if (query.q !== urlQ) {
    setUrlQ(query.q);
    setQ(query.q ?? "");
  }

  const list = useQuery({
    queryKey: ["list", model.name, search.toString()],
    queryFn: () => api.list(model.name, query),
    placeholderData: keepPreviousData,
  });
  // Counted once per search and filters, not on every page.
  const count = useQuery({
    queryKey: ["count", model.name, listSearch({ q: query.q, filters: query.filters })],
    queryFn: () => api.count(model.name, query),
    placeholderData: keepPreviousData,
  });

  /** Apply `changes`, back on the first page unless they name another. */
  const update = (changes: Partial<ListQuery>) => {
    const next = listSearch({
      ...query,
      page: undefined,
      after: undefined,
      before: undefined,
      ...changes,
    });
    setSearch(new URLSearchParams(next.slice(1)));
  };

  const refresh = () => {
    setSelection({ scope, keys: new Set() });
    void queryClient.invalidateQueries({ queryKey: ["list", model.name] });
    void queryClient.invalidateQueries({ queryKey: ["count", model.name] });
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
  // The record's name (or title, email...) links to it; without one, the first column does.
  const linkField = columns.find((f) => f.name === model.display_field) ?? columns[0];
  const filters = model.list_filter
    .map((name) => model.fields.find((f) => f.name === name))
    .filter((f): f is FieldMeta => Boolean(f));
  const sortField = query.sort ?? model.ordering.field;
  const sortOrder = query.order ?? (query.sort ? "asc" : model.ordering.direction);
  const selectable = model.capabilities.delete || model.actions.length > 0;
  const page = list.data;
  const records = page?.records ?? [];
  // Numbered pages count exactly, so the count gives the last page.
  const total = count.data && !count.isPlaceholderData ? count.data.count : undefined;
  const pages =
    page && total?.kind === "exact" ? Math.max(1, Math.ceil(total.value / page.per_page)) : undefined;
  // While the next page or filter loads, the previous rows stay on screen;
  // they cannot be selected, and only rows on screen are ever acted on.
  const stale = list.isPlaceholderData;
  const chosen = stale ? [] : records.filter((r) => selected.has(r.key)).map((r) => r.key);
  const allSelected = !stale && records.length > 0 && records.every((r) => selected.has(r.key));

  const toggle = (key: string) => {
    const keys = new Set(selected);
    if (keys.has(key)) keys.delete(key);
    else keys.add(key);
    setSelection({ scope, keys });
  };
  const toggleAll = () => {
    if (stale) return;
    setSelection({ scope, keys: allSelected ? new Set() : new Set(records.map((r) => r.key)) });
  };
  const sortBy = (name: string) => {
    const order = sortField === name && sortOrder === "asc" ? "desc" : "asc";
    update({ sort: name, order });
  };
  const openRow = (event: MouseEvent<HTMLTableRowElement>, key: string) => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    if ((event.target as Element).closest(INTERACTIVE)) return;
    if (window.getSelection()?.toString()) return;
    navigate(paths.record(model.name, key));
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
          {/* A previous search's count would not match the rows on screen. */}
          {count.data && !count.isPlaceholderData && (
            <p className="text-sm text-slate-500" title={countNote(count.data.count)}>
              {formatCount(count.data.count)} total
            </p>
          )}
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

      {chosen.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 rounded-md bg-indigo-50 px-4 py-2 text-sm">
          <span className="font-medium">{chosen.length} selected</span>
          {model.actions.map((action) => (
            <Button
              key={action.name}
              disabled={runAction.isPending}
              onClick={() => runAction.mutate({ action: action.name, keys: chosen })}
            >
              {action.label}
            </Button>
          ))}
          {model.capabilities.delete && (
            <Button
              variant="danger"
              disabled={bulkDelete.isPending}
              onClick={() => {
                if (window.confirm(`Delete ${chosen.length} selected records?`)) {
                  bulkDelete.mutate(chosen);
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
          <table className="min-w-full divide-y divide-slate-200 text-sm">
            <thead className="bg-slate-50 text-left text-xs font-semibold tracking-wide text-slate-500 uppercase">
              <tr>
                {selectable && (
                  <th className="w-10 px-4 py-2">
                    <input
                      type="checkbox"
                      aria-label="Select all"
                      checked={allSelected}
                      disabled={stale}
                      onChange={toggleAll}
                    />
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
                    <button type="button" className="uppercase hover:text-slate-900" onClick={() => sortBy(field.name)}>
                      {field.label}
                      {sortField === field.name && (sortOrder === "asc" ? " ▲" : " ▼")}
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="divide-y divide-slate-100">
              {records.length === 0 && (
                <tr>
                  <td colSpan={columns.length + 1} className="px-4 py-8 text-center text-slate-500">
                    No records found.
                  </td>
                </tr>
              )}
              {records.map((record) => (
                <tr
                  key={record.key}
                  className="cursor-pointer hover:bg-slate-50"
                  onClick={(event) => openRow(event, record.key)}
                >
                  {selectable && (
                    <td className="p-0" data-select>
                      <label className="flex px-4 py-2">
                        <input
                          type="checkbox"
                          aria-label={`Select ${record.label}`}
                          checked={!stale && selected.has(record.key)}
                          disabled={stale}
                          onChange={() => toggle(record.key)}
                        />
                      </label>
                    </td>
                  )}
                  {columns.map((field) => (
                    <td key={field.name} className="px-4 py-2 whitespace-nowrap">
                      <Cell
                        model={model}
                        models={models}
                        field={field}
                        record={record}
                        labels={page?.labels ?? {}}
                        link={field === linkField}
                      />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}

      {page && page.page !== null && (page.prev || page.next) && (
        <NumberedPages
          page={page.page}
          pages={pages}
          hasNext={Boolean(page.next)}
          onPage={(n) => update({ page: n })}
        />
      )}
      {page && page.page === null && (page.prev || page.next) && (
        <nav aria-label="Pagination" className="flex justify-end gap-2 text-sm">
          <Button disabled={!page.prev} onClick={() => update({ before: page.prev ?? undefined })}>
            Previous
          </Button>
          <Button disabled={!page.next} onClick={() => update({ after: page.next ?? undefined })}>
            Next
          </Button>
        </nav>
      )}
    </div>
  );
}
