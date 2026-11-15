import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useEffect, useRef, useState } from "react";
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

/** Inputs whose browser reports text it cannot read, such as half a date, as empty. */
const PARSED_BY_BROWSER = ["date", "time", "date_time"];

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
  /** A submit was just rejected, by the form or the server. */
  const rejected = useRef(false);
  // The record changed on the server since the form took its values: it was
  // opened from the cache and the fresh copy has arrived, or it was fetched
  // again. Fields left as they were follow it; edited ones keep their text.
  const [synced, setSynced] = useState(data);
  if (data !== synced) {
    const next = initialTexts(fields, data);
    const nextLabels = initialLabels(fields, data);
    const edited = (name: string) => texts[name] !== initial[name];
    setSynced(data);
    setInitial(next);
    setTexts(
      Object.fromEntries(
        fields.map((f) => [f.name, (edited(f.name) ? texts[f.name] : next[f.name]) ?? ""]),
      ),
    );
    setLabels(
      Object.fromEntries(
        fields.flatMap((f) => {
          const label = edited(f.name) ? labels[f.name] : nextLabels[f.name];
          return label ? [[f.name, label]] : [];
        }),
      ),
    );
  }
  const canEdit = isNew ? model.capabilities.create : model.capabilities.update;
  const editable = (field: FieldMeta) =>
    canEdit && !field.readonly && (isNew || !field.primary_key);

  const save = useMutation({
    mutationFn: (values: Record<string, JsonValue>) =>
      data ? api.update(model.name, data.record.key, values) : api.create(model.name, values),
    onSuccess: (res) => {
      setErrors({});
      void queryClient.invalidateQueries({ queryKey: ["list", model.name] });
      void queryClient.invalidateQueries({ queryKey: ["count", model.name] });
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
      // A failure that names no fields, such as a lost connection, leaves
      // none marked from an earlier attempt.
      setErrors(err instanceof ApiError ? err.fields : {});
      rejected.current = true;
    },
  });

  // Rejected fields are described on their inputs, so moving to the first
  // one after a failed submit tells a screen reader user what to fix. Inputs
  // are disabled while saving, so this waits for the save to end.
  const firstInvalid = fields.find((f) => (errors[f.name] ?? []).length > 0)?.name;
  const saving = save.isPending;
  useEffect(() => {
    if (!rejected.current || saving) return;
    rejected.current = false;
    if (firstInvalid) document.getElementById(`field-${firstInvalid}`)?.focus();
  });

  const remove = useMutation({
    mutationFn: () => api.remove(model.name, data?.record.key ?? ""),
    onSuccess: () => {
      notify("Deleted");
      void queryClient.invalidateQueries({ queryKey: ["list", model.name] });
      void queryClient.invalidateQueries({ queryKey: ["count", model.name] });
      void queryClient.invalidateQueries({ queryKey: ["stats"] });
      navigate(paths.list(model.name), { replace: true });
    },
    onError: (err) => notify(err.message, "error"),
  });

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    // An earlier attempt's error would outlive what it was about.
    save.reset();
    const values: Record<string, JsonValue> = {};
    const clientErrors: Record<string, string[]> = {};
    for (const field of fields) {
      if (!editable(field)) continue;
      // By id: an input named after a form property, such as `elements`,
      // would shadow it on the form.
      const input = document.getElementById(`field-${field.name}`);
      if (
        PARSED_BY_BROWSER.includes(field.kind) &&
        input instanceof HTMLInputElement &&
        input.validity.badInput
      ) {
        clientErrors[field.name] = ["is incomplete or not valid"];
        continue;
      }
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
      rejected.current = true;
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
          const required = !field.nullable && field.kind !== "boolean";
          const errorsId = `field-${field.name}-errors`;
          return (
            <div key={field.name} className="grid gap-2 px-5 py-4 sm:grid-cols-[12rem_1fr] sm:items-start">
              <div className="pt-1.5 text-sm font-medium text-slate-700">
                <label htmlFor={`field-${field.name}`}>{field.label}</label>
                {required && editable(field) && (
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
                    aria={{
                      "aria-required": required || undefined,
                      "aria-invalid": fieldErrors.length > 0 || undefined,
                      "aria-describedby": fieldErrors.length > 0 ? errorsId : undefined,
                    }}
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
                {fieldErrors.length > 0 && (
                  <div id={errorsId} className="text-sm text-rose-600">
                    {fieldErrors.map((message) => (
                      <p key={message}>{message}</p>
                    ))}
                  </div>
                )}
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
