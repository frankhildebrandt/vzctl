import type { MessageKey, MessageParams, TFunction } from "@/lib/i18n";
import type { ValidationIssue } from "@/application/validation/topology";

export function formatValidationIssue(
  issue: ValidationIssue,
  t: TFunction,
): string {
  const key = `topo.issue.${issue.code}` as MessageKey;
  const translated = t(key, issue.params);
  if (translated.startsWith("[missing:")) return issue.message;
  return translated;
}

export function validationIssue(
  partial: Omit<ValidationIssue, "message"> & {
    message?: string;
    params?: MessageParams;
  },
): ValidationIssue {
  const { params, message, ...rest } = partial;
  return {
    ...rest,
    params,
    message: message ?? rest.code,
  };
}
