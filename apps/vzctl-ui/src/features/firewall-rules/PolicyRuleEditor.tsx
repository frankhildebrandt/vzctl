import { useForm, useFieldArray } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect } from "react";
import type { AllowRule, Protocol } from "@/domain/hypernetwork/schema";

const RuleFormSchema = z.object({
  rules: z.array(
    z
      .object({
        to: z.string().min(1, "Zielnetz wählen"),
        proto: z.enum(["tcp", "udp", "icmp"]),
        portsText: z.string(),
      })
      .superRefine((val, ctx) => {
        if (val.proto === "icmp") {
          if (val.portsText.trim()) {
            ctx.addIssue({
              code: "custom",
              message: "ICMP ohne Ports",
              path: ["portsText"],
            });
          }
          return;
        }
        const ports = parsePorts(val.portsText);
        if (ports.length === 0) {
          ctx.addIssue({
            code: "custom",
            message: "Mindestens ein Port (1–65535)",
            path: ["portsText"],
          });
        }
      }),
  ),
});

type RuleForm = z.infer<typeof RuleFormSchema>;

function parsePorts(text: string): number[] {
  return text
    .split(/[,\s]+/)
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => Number(s))
    .filter((n) => Number.isInteger(n) && n >= 1 && n <= 65535);
}

function portsToText(ports: number[]): string {
  return ports.join(", ");
}

type Props = {
  policyName: string;
  networkName: string;
  networks: string[];
  allow: AllowRule[];
  onChange: (allow: AllowRule[]) => void;
};

export function PolicyRuleEditor({
  policyName,
  networkName,
  networks,
  allow,
  onChange,
}: Props) {
  const form = useForm<RuleForm>({
    resolver: zodResolver(RuleFormSchema),
    defaultValues: {
      rules: allow.map((r) => ({
        to: r.to,
        proto: r.proto,
        portsText: portsToText(r.ports),
      })),
    },
  });

  const { fields, append, remove, move } = useFieldArray({
    control: form.control,
    name: "rules",
  });

  useEffect(() => {
    form.reset({
      rules: allow.map((r) => ({
        to: r.to,
        proto: r.proto,
        portsText: portsToText(r.ports),
      })),
    });
  }, [allow, form, policyName]);

  const submit = form.handleSubmit((data) => {
    const next: AllowRule[] = data.rules.map((r) => ({
      to: r.to,
      proto: r.proto as Protocol,
      ports: r.proto === "icmp" ? [] : parsePorts(r.portsText),
    }));
    onChange(next);
  });

  return (
    <div className="policy-editor">
      <div className="row" style={{ justifyContent: "space-between" }}>
        <h4>
          Policy <code>{policyName}</code>
        </h4>
        <button
          type="button"
          className="secondary"
          onClick={() =>
            append({
              to:
                networks.find((n) => n !== networkName) ??
                "internet",
              proto: "tcp",
              portsText: "80",
            })
          }
        >
          Regel +
        </button>
      </div>
      <p className="muted">
        Netzwerk <strong>{networkName}</strong> · forward: deny-all
      </p>
      <form onSubmit={submit} className="policy-form">
        {fields.length === 0 ? (
          <p className="muted">Keine Allow-Regeln.</p>
        ) : (
          <ol className="policy-rule-list">
            {fields.map((field, index) => (
              <li key={field.id} className="policy-rule">
                <div className="policy-rule-head">
                  <span>#{index + 1}</span>
                  <div className="row" style={{ gap: "0.35rem" }}>
                    <button
                      type="button"
                      className="secondary"
                      disabled={index === 0}
                      aria-label="Nach oben"
                      onClick={() => move(index, index - 1)}
                    >
                      ↑
                    </button>
                    <button
                      type="button"
                      className="secondary"
                      disabled={index === fields.length - 1}
                      aria-label="Nach unten"
                      onClick={() => move(index, index + 1)}
                    >
                      ↓
                    </button>
                    <button
                      type="button"
                      className="secondary"
                      aria-label="Duplizieren"
                      onClick={() => {
                        const current = form.getValues(`rules.${index}`);
                        append({ ...current });
                      }}
                    >
                      Duplizieren
                    </button>
                    <button
                      type="button"
                      className="secondary"
                      aria-label="Löschen"
                      onClick={() => remove(index)}
                    >
                      Löschen
                    </button>
                  </div>
                </div>
                <label className="topology-field">
                  <span>Ziel</span>
                  <select {...form.register(`rules.${index}.to`)}>
                    <option value="internet">internet</option>
                    {networks
                      .filter((n) => n !== networkName)
                      .map((n) => (
                        <option key={n} value={n}>
                          {n}
                        </option>
                      ))}
                    <option value={networkName}>{networkName} (self)</option>
                  </select>
                </label>
                <label className="topology-field">
                  <span>Protokoll</span>
                  <select {...form.register(`rules.${index}.proto`)}>
                    <option value="tcp">tcp</option>
                    <option value="udp">udp</option>
                    <option value="icmp">icmp</option>
                  </select>
                </label>
                <label className="topology-field">
                  <span>Ports</span>
                  <input
                    {...form.register(`rules.${index}.portsText`)}
                    placeholder="80, 443"
                    aria-invalid={
                      Boolean(form.formState.errors.rules?.[index]?.portsText)
                    }
                  />
                </label>
                {form.formState.errors.rules?.[index]?.portsText ? (
                  <p className="field-error" role="alert">
                    {form.formState.errors.rules[index]?.portsText?.message}
                  </p>
                ) : null}
                {form.formState.errors.rules?.[index]?.to ? (
                  <p className="field-error" role="alert">
                    {form.formState.errors.rules[index]?.to?.message}
                  </p>
                ) : null}
              </li>
            ))}
          </ol>
        )}
        <button type="submit">Regeln übernehmen</button>
      </form>
    </div>
  );
}
