// ui-host: boots a site's page. It asks the gateway for this viewer's manifest (ui.manifest Resolve: only the views
// the viewer's rights may receive), fetches each module from ui.serve by its sha256 name, checks the bytes against
// the manifest's digest before importing (a changed byte is refused and never runs), mounts the root view and fills
// each slot with the module the manifest placed there. A slot the manifest has no module for stays empty; the host
// knows nothing else about it. A panel whose backend refuses the grant (ended, revoked) is taken off the page, and the
// manifest is asked again every 30 s, so a right that is taken away removes its panel from an open page.
import { errText, type Grant, type Host, type ViewModule } from './ui-rt';

export interface Module { api: string; major: number; view: string; slot: string; file: string; sha256: string }
export interface Manifest { site: string; route: string; modules: Module[] }

export interface Boot {
  readonly grant: Grant;
  /** this page's manifest (live: GET manifest.json) */
  manifest(): Promise<Manifest>;
  /** the module's bytes and its import (live: fetch + import of the same URL) */
  fetch(file: string): Promise<ArrayBuffer>;
  load(file: string): Promise<unknown>;
  /** a refused module is reported, not run */
  refused(m: Module, why: string): void;
}

const hex = (b: ArrayBuffer): string => Array.from(new Uint8Array(b), (x) => x.toString(16).padStart(2, '0')).join('');

export async function digest(b: ArrayBuffer): Promise<string> {
  return hex(await crypto.subtle.digest('SHA-256', b));
}

function isViewModule(m: unknown): m is { default: ViewModule } {
  if (typeof m !== 'object' || m === null || !('default' in m)) return false;
  const d: unknown = m.default;
  return typeof d === 'object' && d !== null && 'mount' in d && typeof d.mount === 'function';
}

/** Fetch a module, check its digest, import it. Throws on a digest that differs. */
export async function verified(boot: Boot, m: Module): Promise<ViewModule> {
  const bytes = await boot.fetch(m.file);
  const got = await digest(bytes);
  if (got !== m.sha256) {
    const why = `integrity: ${m.file} has sha256 ${got.slice(0, 16)}.., the manifest says ${m.sha256.slice(0, 16)}..; not run`;
    boot.refused(m, why);
    throw new Error(why);
  }
  const mod = await boot.load(m.file);
  if (!isViewModule(mod)) throw new Error(`${m.file} is not a view module`);
  return mod.default;
}

interface Mounted { m: Module; el: HTMLElement; stop: () => void }

export async function start(boot: Boot, root: HTMLElement, onEvent: (from: Module, event: string, arg: unknown) => void = () => {}): Promise<void> {
  let manifest = await boot.manifest();
  const mounted = new Map<string, Mounted>();
  const slotEls = new Map<string, HTMLElement>();

  const unmount = (slot: string, why: string): void => {
    for (const [s, x] of Array.from(mounted.entries())) {
      if (s === slot || s.startsWith(slot === '' ? '' : slot + '/')) {
        x.stop();
        x.el.replaceChildren();
        x.el.dataset['state'] = 'empty';
        mounted.delete(s);
      }
    }
    if (slot === '') root.replaceChildren(face(`This page is closed for you: ${why}`));
  };

  const mountAt = async (m: Module, el: HTMLElement): Promise<void> => {
    const prev = mounted.get(m.slot);
    if (prev !== undefined && prev.m.sha256 === m.sha256) return;
    prev?.stop();
    let view: ViewModule;
    try {
      view = await verified(boot, m);
    } catch (e) {
      el.replaceChildren(face(errText(e)));
      return;
    }
    const host: Host<unknown> = {
      props: {},
      emit: (event, arg) => onEvent(m, event, arg),
      slot: (name) => {
        const path = m.slot === '' ? name : `${m.slot}/${name}`;
        let s = slotEls.get(path);
        if (s === undefined) {
          s = document.createElement('div');
          s.className = 'slot';
          s.dataset['state'] = 'empty';
          slotEls.set(path, s);
          const child = manifest.modules.find((x) => x.slot === path);
          if (child !== undefined) void mountAt(child, s);
        }
        return s;
      },
      gone: (why) => unmount(m.slot, why),
    };
    el.dataset['state'] = 'mounted';
    const stop = view.mount(el, boot.grant, host);
    mounted.set(m.slot, { m, el, stop });
  };

  const rootModule = manifest.modules.find((m) => m.slot === '');
  if (rootModule === undefined) {
    root.replaceChildren(face('Nothing here for your sign-in.'));
    return;
  }
  await mountAt(rootModule, root);

  // the manifest again every 30 s: a module that left it leaves the page, one that joined it is mounted
  setInterval(() => {
    boot.manifest().then((next) => {
      manifest = next;
      for (const slot of Array.from(mounted.keys())) {
        if (!next.modules.some((m) => m.slot === slot)) unmount(slot, 'no longer in your grant');
      }
      for (const m of next.modules) {
        const el = m.slot === '' ? root : slotEls.get(m.slot);
        if (el !== undefined && !mounted.has(m.slot)) void mountAt(m, el);
      }
    }, () => { /* the gateway did not answer: keep what is mounted */ });
  }, 30_000);
}

function face(text: string): HTMLElement {
  const d = document.createElement('div');
  d.className = 'face bad';
  d.textContent = text;
  return d;
}

/** The live boot: the gateway's manifest and ui.serve, same origin as the page. */
export function liveBoot(grant: Grant): Boot {
  return {
    grant,
    async manifest() {
      const r = await fetch('manifest.json', { cache: 'no-store', credentials: 'same-origin' });
      if (!r.ok) throw new Error(`manifest: ${r.status}`);
      const m: unknown = await r.json();
      if (typeof m !== 'object' || m === null || !('modules' in m) || !Array.isArray(m.modules)) throw new Error('manifest: not a manifest');
      return m as Manifest;
    },
    async fetch(file) {
      const r = await fetch(file, { credentials: 'same-origin' });
      if (!r.ok) throw new Error(`${file}: ${r.status} ${await r.text()}`);
      return r.arrayBuffer();
    },
    load: (file) => import(/* @vite-ignore */ new URL(file, location.href).href),
    refused: (m, why) => console.error(`ui-host refused ${m.api}@${m.major}/${m.view}: ${why}`),
  };
}
