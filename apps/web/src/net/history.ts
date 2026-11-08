// Commits of an agent's workspace as the node returns them (`git.log`, ADR-0037).
export interface CommitItem { id: string; subject: string; author: string; atMs: number }

type Raw = { id?: string | null; subject?: string | null; author?: string | null; atMs?: number | bigint | { toString(): string } | null };

export function commitsOf(raw: readonly Raw[] | undefined | null): CommitItem[] {
  return (raw ?? []).map((c) => ({ id: c.id ?? '', subject: c.subject ?? '', author: c.author ?? '', atMs: Number(c.atMs?.toString() ?? 0) }));
}
