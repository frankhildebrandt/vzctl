export type ApplyRecovery = {
  kind: "journal" | "config-changed" | "lease";
  holder?: string;
  pid?: number;
  expiresAt?: string;
  journalId?: string;
  step?: string;
};

/** Decode the stable supervisor error text until a structured REST error exists. */
export function parseApplyRecovery(raw: string): ApplyRecovery | null {
  const lease = /stack lease held by (\S+) until ([^\s"\\]+)/.exec(raw);
  if (lease) {
    const pid = /:(\d+)$/.exec(lease[1]);
    return { kind: "lease", holder: lease[1], expiresAt: lease[2], ...(pid ? { pid: Number(pid[1]) } : {}) };
  }
  if (raw.includes("stack lease was lost") || raw.includes("apply lease renewal failed")) return { kind: "lease" };
  if (raw.includes("desired config changed") && raw.includes("--abort")) return { kind: "config-changed" };
  const journal = /incomplete journal ([\w-]+) at ([\w-]+)/.exec(raw);
  if (journal) return { kind: "journal", journalId: journal[1], step: journal[2] };
  if (!raw.includes("no incomplete journal") && raw.includes("incomplete journal")) return { kind: "journal" };
  return null;
}
