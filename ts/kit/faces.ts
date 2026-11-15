// The faces every view draws for the states of its data, and the line that says where the rows came from.
import { ago, exhaustive, h } from './dom';

export type SourceStateLike =
  | { tag: 'Live' }
  | { tag: 'Stale'; age_s: bigint }
  | { tag: 'Down'; why: string }
  | { tag: 'Refused'; needs: string };
export interface SourceLike { readonly cell: string; readonly node: string; readonly state: SourceStateLike; readonly ms: number }

export function loading(): HTMLElement {
  return h('div', { class: 'face', role: 'status', text: 'Asking the cell...' });
}

export function empty(text: string): HTMLElement {
  return h('div', { class: 'face', text });
}

export function failed(why: string): HTMLElement {
  return h('div', { class: 'face bad' }, h('b', { text: 'No answer. ' }), why);
}

export function stateText(s: SourceStateLike): [string, string] {
  switch (s.tag) {
    case 'Live':
      return ['live', 'g'];
    case 'Stale':
      return [`${s.age_s} s old`, ''];
    case 'Down':
      return ['no answer', 'r'];
    case 'Refused':
      return [`refused: needs ${s.needs}`, 'r'];
    default:
      return exhaustive(s);
  }
}

/** The line under every panel: how each source answered. */
export function sourcesLine(sources: readonly SourceLike[] | undefined, live: boolean): HTMLElement {
  const items = (sources ?? []).map((s) => {
    const [t, c] = stateText(s.state);
    const why = s.state.tag === 'Down' ? s.state.why : '';
    return h('span', { class: 'src', title: why }, h('i', { class: c }), `${s.cell}${s.node ? ' @ ' + s.node : ''}: ${t}`);
  });
  return h('div', { class: 'srcs' }, h('span', { class: 'src' }, h('i', { class: live ? 'g' : '' }), live ? 'live: rows update by themselves' : 'connecting'), ...items);
}

export { ago };
