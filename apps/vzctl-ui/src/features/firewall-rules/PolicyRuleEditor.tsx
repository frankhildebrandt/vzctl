import { useForm, useFieldArray } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useMemo } from "react";
import {
  ActionRow,
  Button,
  FieldError,
  FormField,
  Muted,
} from "@/components/ui";
import type { AllowRule, Protocol } from "@/domain/hypernetwork/schema";
import { useT } from "@/lib/i18n";

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
  const t = useT();

  const RuleFormSchema = useMemo(
    () =>
      z.object({
        rules: z.array(
          z
            .object({
              to: z.string().min(1, t("firewall.error.target")),
              proto: z.enum(["tcp", "udp", "icmp"]),
              portsText: z.string(),
            })
            .superRefine((val, ctx) => {
              if (val.proto === "icmp") {
                if (val.portsText.trim()) {
                  ctx.addIssue({
                    code: "custom",
                    message: t("firewall.error.icmpNoPorts"),
                    path: ["portsText"],
                  });
                }
                return;
              }
              const ports = parsePorts(val.portsText);
              if (ports.length === 0) {
                ctx.addIssue({
                  code: "custom",
                  message: t("firewall.error.portRequired"),
                  path: ["portsText"],
                });
              }
            }),
        ),
      }),
    [t],
  );

  type RuleForm = z.infer<typeof RuleFormSchema>;

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
      <ActionRow align="between">
        <h4>{t("firewall.policyTitle", { name: policyName })}</h4>
        <Button
          tone="secondary"
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
          {t("firewall.addRule")}
        </Button>
      </ActionRow>
      <Muted>
        {t("firewall.networkForward", { name: networkName })}
      </Muted>
      <form onSubmit={submit} className="policy-form">
        {fields.length === 0 ? (
          <p className="muted">{t("firewall.noRules")}</p>
        ) : (
          <ol className="policy-rule-list">
            {fields.map((field, index) => (
              <li key={field.id} className="policy-rule">
                <div className="policy-rule-head">
                  <span>{t("firewall.ruleN", { n: index + 1 })}</span>
                  <ActionRow gap="sm">
                    <Button
                      tone="secondary"
                      disabled={index === 0}
                      aria-label={t("firewall.moveUp")}
                      onClick={() => move(index, index - 1)}
                    >
                      ↑
                    </Button>
                    <Button
                      tone="secondary"
                      disabled={index === fields.length - 1}
                      aria-label={t("firewall.moveDown")}
                      onClick={() => move(index, index + 1)}
                    >
                      ↓
                    </Button>
                    <Button
                      tone="secondary"
                      aria-label={t("firewall.duplicate")}
                      onClick={() => {
                        const current = form.getValues(`rules.${index}`);
                        append({ ...current });
                      }}
                    >
                      {t("firewall.duplicate")}
                    </Button>
                    <Button
                      tone="secondary"
                      aria-label={t("common.delete")}
                      onClick={() => remove(index)}
                    >
                      {t("common.delete")}
                    </Button>
                  </ActionRow>
                </div>
                <FormField label={t("firewall.target")} variant="compact">
                  <select {...form.register(`rules.${index}.to`)}>
                    <option value="internet">{t("firewall.targetInternet")}</option>
                    {networks
                      .filter((n) => n !== networkName)
                      .map((n) => (
                        <option key={n} value={n}>
                          {n}
                        </option>
                      ))}
                    <option value={networkName}>
                      {t("firewall.targetSelf", { name: networkName })}
                    </option>
                  </select>
                </FormField>
                <FormField label={t("firewall.proto")} variant="compact">
                  <select {...form.register(`rules.${index}.proto`)}>
                    <option value="tcp">tcp</option>
                    <option value="udp">udp</option>
                    <option value="icmp">icmp</option>
                  </select>
                </FormField>
                <FormField label={t("firewall.ports")} variant="compact">
                  <input
                    {...form.register(`rules.${index}.portsText`)}
                    placeholder={t("firewall.portsPlaceholder")}
                    aria-invalid={
                      Boolean(form.formState.errors.rules?.[index]?.portsText)
                    }
                  />
                </FormField>
                <FieldError
                  className="field-error"
                  message={form.formState.errors.rules?.[index]?.portsText?.message}
                />
                <FieldError
                  className="field-error"
                  message={form.formState.errors.rules?.[index]?.to?.message}
                />
              </li>
            ))}
          </ol>
        )}
        <Button type="submit">{t("firewall.applyRules")}</Button>
      </form>
    </div>
  );
}
