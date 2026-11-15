// The shared kit: a typed element builder and the pieces every panel draws with. No framework, no per-panel CSS.
export type Kid = Node | string | null | false | undefined;
export interface Props {
  class?: string;
  text?: string;
  title?: string;
  role?: string;
  label?: string;
  pressed?: boolean;
  disabled?: boolean;
  onclick?: (e: Event) => void;
  oninput?: (e: Event) => void;
  attrs?: Readonly<Record<string, string>>;
}

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, p: Props = {}, ...kids: Kid[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (p.class !== undefined) el.className = p.class;
  if (p.text !== undefined) el.textContent = p.text;
  if (p.title !== undefined) el.title = p.title;
  if (p.role !== undefined) el.setAttribute('role', p.role);
  if (p.label !== undefined) el.setAttribute('aria-label', p.label);
  if (p.pressed !== undefined) el.setAttribute('aria-pressed', String(p.pressed));
  if (p.disabled === true) el.setAttribute('disabled', '');
  if (p.onclick !== undefined) el.addEventListener('click', p.onclick);
  if (p.oninput !== undefined) el.addEventListener('input', p.oninput);
  for (const [k, v] of Object.entries(p.attrs ?? {})) el.setAttribute(k, v);
  for (const k of kids) if (k !== null && k !== false && k !== undefined) el.append(k);
  return el;
}

export function clear(el: Element, ...kids: Kid[]): void {
  el.replaceChildren();
  for (const k of kids) if (k !== null && k !== false && k !== undefined) el.append(k);
}

/** A compile error when a union gains a case a switch does not draw. */
export function exhaustive(x: never): never {
  throw new Error(`unhandled case ${JSON.stringify(x)}`);
}

const n = (v: bigint | number): number => (typeof v === 'bigint' ? Number(v) : v);

/** "4 s ago", "3 min ago", "2 h ago", "5 d ago" from a time in ms. */
export function ago(atMs: bigint | number, nowMs: number = Date.now()): string {
  const t = n(atMs);
  if (t <= 0) return 'never';
  const s = Math.max(0, Math.round((nowMs - t) / 1000));
  if (s < 60) return `${s} s ago`;
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

export function date(atMs: bigint | number): string {
  const t = n(atMs);
  if (t <= 0) return '-';
  return new Date(t).toISOString().slice(0, 10);
}

export function time(atMs: bigint | number): string {
  const t = n(atMs);
  if (t <= 0) return '-';
  return new Date(t).toISOString().replace('T', ' ').slice(0, 19);
}

export function num(v: bigint | number): string {
  return n(v).toLocaleString('en-US');
}

export { n as toNumber };
