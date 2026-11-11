import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Link, useNavigate } from "react-router";
import { ApiError, api } from "../api";
import { InputError, displayFull, fromInput, keyText, toInput } from "../format";
import { paths } from "../paths";
import type { FieldMeta, JsonValue, ModelMeta, RecordResponse } from "../types";
import { FieldInput } from "./FieldInput";
import { Button, Card, ErrorBanner, Spinner, useToast } from "./ui";

type Texts = Record<string, string>;

function initialTexts(fields: FieldMeta[], data?: RecordResponse): Texts {
  const texts: Texts = {};
  for (const field of fields) {
    const value = data?.record.values[field.name];
    texts[field.name] =
      data || field.kind !== "boolean" || field.nullable ? toInput(field, value) : "false";
  }
  return texts;
}

function initialLabels(fields: FieldMeta[], data?: RecordResponse): Texts {
  const labels: Texts = {};
  for (const field of fields) {
    const value = data?.record.values[field.name];
    const label = data?.labels[field.name]?.[keyText(value)];
    if (label) labels[field.name] = label;
  }
  return labels;
}

function RecordForm({
  model,
  models,
  data,
}: {
  model: ModelMeta;
  models: ModelMeta[];
  data?: RecordResponse;
}) {
  const isNew = !data;
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { notify } = useToast();
  const fields = isNew ? model.fields.filter((f) => !f.auto_increment && !f.readonly) : model.fields;
  const [initial, setInitial] = useState(() => initialTexts(fields, data));
  const [texts, setTexts] = useState<Texts>(initial);
  const [labels, setLabels] = useState<Texts>(() => initialLabels(fields, data));
  const [errors, setErrors] = useState<Record<string, string[]>>({});
  const canEdit = isNew ? model.capabilities.create : model.capabilities.update;
  const editable = (field: FieldMeta) =>
    canEdit && !field.readonly && (isNew || !field.primary_key);

  const save = useMutation({
    mutationFn: (values: Record<string, JsonValue>) =>
      data ? api.update(model.name, data.record.key, values) : api.create(model.name, values),
    onSuccess: (res) => {
      setErrors({});
      void queryClient.invalidateQueries({ queryKey: ["list", model.name] });
      void queryClient.invalidateQueries({ queryKey: ["stats"] });
      queryClient.setQueryData(["record", model.name, res.record.key], res);
      if (isNew) {
        notify(`Created ${res.record.label}`);
        navigate(paths.record(model.name, res.record.key), { replace: true });
      } else {
        notify("Saved");
        const next = initialTexts(fields, res);
        setInitial(next);
        setTexts(next);
      }
    },
    onError: (err) => {
      if (err instanceof ApiError) setErrors(err.fields);
    },
  });

  const remove = useMutation({
    mutationFn: () => api.remove(model.name, data?.record.key ?? ""),
    onSuccess: () => {
      notify("Deleted");
      void queryClient.invalidateQueries({ queryKey: ["list", model.name] });
      void queryClient.invalidateQueries({ queryKey: ["stats"] });
      navigate(paths.list(model.name), { replace: true });
    },
    onError: (err) => notify(err.message, "error"),
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const values: Record<string, JsonValue> = {};
    const clientErrors: Record<string, string[]> = {};
    for (const field of fields) {
      if (!editable(field)) continue;
      const text = texts[field.name] ?? "";
      if (isNew ? text === "" : text === initial[field.name]) continue;
      try {
        values[field.name] = fromInput(field, text);
      } catch (err) {
        clientErrors[field.name] = [err instanceof InputError ? err.message : String(err)];
      }
    }
    if (Object.keys(clientErrors).length > 0) {
      setErrors(clientErrors);
      return;
    }
    save.mutate(values);
  };

  // Errors for fields the form does not show (read-only on create, or a
  // model-wide `__all__`) would otherwise be lost.
  const otherErrors = Object.entries(errors).flatMap(([name, messages]) => {
    if (fields.some((f) => f.name === name)) return [];
    const label = model.fields.find((f) => f.name === name)?.label;
    return messages.map((message) => (name === "__all__" || !label ? message : `${label}: ${message}`));
  });

  const related = data
    ? model.related.flatMap((rel) => {
        const child = models.find((m) => m.name === rel.model);
        const column = child?.fields.find((f) => f.name === rel.field)?.relation?.column;
        const value = column ? data.record.values[column] : undefined;
        if (!child || value === undefined || value === null) return [];
        const search = `?filter.${encodeURIComponent(rel.field)}=${encodeURIComponent(keyText(value))}`;
        return [{ ...rel, to: paths.list(rel.model, search) }];
      })
    : [];

  return (
    <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <Link to={paths.list(model.name)} className="text-sm text-indigo-600 hover:underline">
            ← {model.label}
          </Link>
          <h1 className="text-2xl font-semibold">
            {data ? data.record.label : "New record"}
          </h1>
        </div>
        <div className="flex gap-2">
          {data && model.capabilities.delete && (
            <Button
              variant="danger"
              disabled={remove.isPending}
              onClick={() => {
                if (window.confirm(`Delete ${data.record.label}?`)) remove.mutate();
              }}
            >
              Delete
            </Button>
          )}
          {canEdit && (
            <Button type="submit" variant="primary" disabled={save.isPending}>
              {save.isPending ? "Saving…" : isNew ? "Create" : "Save"}
            </Button>
          )}
        </div>
      </div>

      {save.error && <ErrorBanner error={save.error} />}
      {otherErrors.length > 0 && (
        <ul role="alert" className="list-disc rounded-md bg-rose-50 py-2 pr-4 pl-8 text-sm text-rose-700">
          {otherErrors.map((message) => (
            <li key={message}>{message}</li>
          ))}
        </ul>
      )}

      <Card className="divide-y divide-slate-100">
        {fields.map((field) => {
          const fieldErrors = errors[field.name] ?? [];
          const value = data?.record.values[field.name];
          return (
            <div key={field.name} className="grid gap-2 px-5 py-4 sm:grid-cols-[12rem_1fr] sm:items-start">
              <div className="pt-1.5 text-sm font-medium text-slate-700">
                <label htmlFor={`field-${field.name}`}>{field.label}</label>
                {!field.nullable && editable(field) && field.kind !== "boolean" && (
                  <span aria-hidden="true" className="text-rose-500">
                    {" "}
                    *
                  </span>
                )}
              </div>
              <div className="flex flex-col gap-1">
                {editable(field) ? (
                  <FieldInput
                    field={field}
                    value={texts[field.name] ?? ""}
                    disabled={save.isPending}
                    fkLabel={labels[field.name]}
                    onChange={(text) => setTexts((t) => ({ ...t, [field.name]: text }))}
                    onPick={(text, label) => {
                      setTexts((t) => ({ ...t, [field.name]: text }));
                      setLabels((l) => ({ ...l, [field.name]: label }));
                    }}
                  />
                ) : (
                  <output id={`field-${field.name}`} className="block pt-1.5 text-sm break-all whitespace-pre-wrap">
                    {field.kind === "json" && value !== null && value !== undefined
                      ? JSON.stringify(value, null, 2)
                      : (labels[field.name] ?? displayFull(field, value))}
                  </output>
                )}
                {fieldErrors.map((message) => (
                  <p key={message} className="text-sm text-rose-600">
                    {message}
                  </p>
                ))}
              </div>
            </div>
          );
        })}
      </Card>

      {related.length > 0 && (
        <Card className="p-5">
          <h2 className="mb-2 text-sm font-semibold text-slate-500 uppercase">Related</h2>
          <ul className="flex flex-wrap gap-3">
            {related.map((rel) => (
              <li key={`${rel.model}.${rel.field}`}>
                <Link to={rel.to} className="text-indigo-600 hover:underline">
                  {rel.label}
                </Link>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </form>
  );
}

export function RecordPage({
  model,
  models,
  recordKey,
}: {
  model: ModelMeta;
  models: ModelMeta[];
  recordKey?: string;
}) {
  const record = useQuery({
    queryKey: ["record", model.name, recordKey],
    queryFn: () => api.get(model.name, recordKey ?? ""),
    enabled: recordKey !== undefined,
  });
  if (recordKey === undefined) {
    if (!model.capabilities.create) {
      return <ErrorBanner error={`You cannot create ${model.label.toLowerCase()}.`} />;
    }
    return <RecordForm model={model} models={models} />;
  }
  if (record.isPending) return <Spinner />;
  if (record.error) return <ErrorBanner error={record.error} />;
  return <RecordForm key={record.data.record.key} model={model} models={models} data={record.data} />;
}
