// ui-rt: the runtime of a full-stack cell's view. A view is declared in its cell's Rust contract (`views!`), bound by
// ui-gen (`ui/gen/<Api>.ui.ts`: `defineView` with typed props, data, events and slots), and written once in
// `ui/src/<View>.ts` with `<View>.render(v => ...)`. The view gets its data only through the generated loaders, which
// call only its own cell's ops; it is never told the viewer's rights (a field the viewer may not have is absent, a
// row is not there, a slot whose cell the viewer may not have stays empty).

/** The browser grant a page holds (the ts-web client's `Grant`, structurally). */
export interface Grant { url: string; seed: Uint8Array; chain: Uint8Array; everyone?: string }

/** A live subscription: the ts-web client's `Subscription<T>` or a sim feed. */
export interface Feed<T> extends AsyncIterable<T> { close(): void }

/** The item a `Watch` stream carries: on `reset` the rows are replaced, else `put` upserts and `del` removes by key. */
export interface Delta<R> { readonly reset: boolean; readonly put: readonly R[]; readonly del: readonly string[]; readonly hlc: bigint }
export type RowOf<D> = D extends { readonly put: readonly (infer R)[] } ? R : never;
export type ItemOf<F> = F extends AsyncIterable<infer T> ? T : never;

/** A `Part<T, Right>` field: absent (null) on the wire for a viewer without the right. No direct value: `show` it. */
export type Part<T> = T | null;
export function show<T, U>(p: Part<T>, f: (v: T) => U): U | null {
  return p === null ? null : f(p);
}

/** What a mounted view may do: read its data and props, emit its declared events, place its declared slots. */
export interface ViewCtx<P, D, E, S extends string> {
  readonly props: Readonly<P>;
  /** Called now and after every data change (batched per animation frame). */
  draw(f: (d: Partial<D>, failed: string | null) => void): void;
  /** New props: the data is loaded again for them. */
  setProps(p: Partial<P>): void;
  readonly emit: E;
  /** The element the host fills with the cell placed in that slot; empty when the viewer may not have that cell. */
  slot(name: S): HTMLElement;
  /**
   * The same for a slot named at run time by this view's own backend (the shell's `Layout` names only the slots the
   * viewer may open, so the bundle never carries the name of one they may not). Its `data-state` is `mounted` or
   * `empty`: the one thing a parent can observe about a slot.
   */
  place(name: string): HTMLElement;
}

/** What the host gives a view it mounts. */
export interface Host<P> {
  readonly props: P;
  emit(event: string, arg: unknown): void;
  slot(name: string): HTMLElement;
  /** The view's backend refused the grant (ended or revoked): the host takes the panel off the page. */
  gone(why: string): void;
}

export interface Loaders<P, D, Src> {
  (src: Src, props: P, set: <K extends keyof D>(k: K, v: D[K]) => void, fail: (why: string) => void, gone: (why: string) => void, again: () => void): () => void;
}

export interface Spec<P, D, E, Src, S extends string> {
  readonly api: string;
  readonly major: number;
  readonly view: string;
  readonly needs: string;
  readonly events: readonly (keyof E & string)[];
  /** every prop's value when the placing view gives none */
  readonly defaults: P;
  readonly connect: (g: Grant) => Promise<Src & { close(): void }>;
  readonly load: Loaders<P, D, Src>;
}

/** A built view module's default export: what ui-host mounts. */
export interface ViewModule {
  readonly api: string;
  readonly major: number;
  readonly view: string;
  mount(el: HTMLElement, grant: Grant, host: Host<unknown>): () => void;
}

export interface View<P, D, E, Src, S extends string> {
  readonly spec: Spec<P, D, E, Src, S>;
  render(f: (v: ViewCtx<P, D, E, S>) => Node): Bound<P, D, E, Src, S>;
}

export interface Bound<P, D, E, Src, S extends string> extends ViewModule {
  /** Mount with a given source (sim mode, tests): the same view code, another source of the same type. */
  mountWith(el: HTMLElement, src: Src, host: Host<P>): () => void;
  readonly spec: Spec<P, D, E, Src, S>;
}

export function defineView<P, D, E, Src, S extends string = never>(spec: Spec<P, D, E, Src, S>): View<P, D, E, Src, S> {
  return {
    spec,
    render(f) {
      const run = (el: HTMLElement, first: Src, host: Host<P>, reconnect: (() => Promise<Src>) | null): (() => void) => {
        let src = first;
        let props: P = { ...spec.defaults, ...host.props };
        let data: Partial<D> = {};
        let failed: string | null = null;
        const draws: ((d: Partial<D>, failed: string | null) => void)[] = [];
        let queued = false;
        const redraw = (): void => {
          if (queued) return;
          queued = true;
          requestAnimationFrame(() => {
            queued = false;
            for (const d of draws) d(data, failed);
          });
        };
        let stop: () => void = () => {};
        let ended = false;
        const gone = (why: string): void => {
          if (ended) return;
          ended = true;
          stop();
          host.gone(why);
        };
        const load = (): void => {
          stop();
          stop = spec.load(src, props, (k, v) => { data = { ...data, [k]: v }; failed = null; redraw(); }, (why) => { failed = why; redraw(); }, gone, again);
        };
        // a stream ended: open a new connection (a new Hello, so an ended or revoked grant is refused now) and load again
        let reopening = false;
        const again = (): void => {
          if (ended || reopening) return;
          if (reconnect === null) return load();
          reopening = true;
          stop();
          reconnect().then(
            (s) => { reopening = false; if (ended) return; src = s; load(); },
            (e: unknown) => { reopening = false; if (isGrantRefusal(e)) gone(errText(e)); else { failed = errText(e); redraw(); setTimeout(again, 2000); } },
          );
        };
        const emitter: Record<string, (a: unknown) => void> = {};
        for (const ev of spec.events) emitter[ev] = (a: unknown) => host.emit(ev, a);
        const slots = new Map<string, HTMLElement>();
        const ctx: ViewCtx<P, D, E, S> = {
          get props() { return props; },
          draw(d) { draws.push(d); d(data, failed); },
          setProps(p) { props = { ...props, ...p }; load(); },
          emit: emitter as unknown as E,
          slot(name) {
            return ctx.place(name);
          },
          place(name) {
            let s = slots.get(name);
            if (s === undefined) { s = host.slot(name); slots.set(name, s); }
            return s;
          },
        };
        el.replaceChildren(f(ctx));
        load();
        return () => { ended = true; stop(); };
      };
      const bound: Bound<P, D, E, Src, S> = {
        api: spec.api,
        major: spec.major,
        view: spec.view,
        spec,
        mountWith: (el, src, host) => run(el, src, host, null),
        mount(el, grant, host) {
          let src: (Src & { close(): void }) | null = null;
          let done = false;
          let unmount: () => void = () => {};
          const connect = async (): Promise<Src> => {
            src?.close();
            const s = await spec.connect(grant);
            if (done) { s.close(); throw new Error('unmounted'); }
            src = s;
            return s;
          };
          void connect().then(
            (s) => { unmount = run(el, s, host as Host<P>, connect); },
            (e: unknown) => host.gone(errText(e)),
          );
          return () => { done = true; unmount(); src?.close(); };
        },
      };
      return bound;
    },
  };
}

export function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** True for a refusal of the grant itself (ended, revoked, never valid): the panel leaves the page. */
export function isGrantRefusal(e: unknown): boolean {
  const kind = typeof e === 'object' && e !== null && 'kind' in e ? String((e as { kind: unknown }).kind) : '';
  return kind === 'Gate' || kind === 'Right' || /Gate|revoked|expired|no grant/i.test(errText(e));
}

/**
 * Load one answer; after `everyMs`, `again` opens a new connection and loads it again (a new Hello each time, so an
 * ended or revoked grant is refused within that time, as for streams).
 */
export function once<T>(run: () => Promise<T>, set: (v: T) => void, fail: (why: string) => void, gone: (why: string) => void, again: () => void, everyMs = 30_000): () => void {
  let live = true;
  run().then((v) => { if (live) set(v); }, (e: unknown) => { if (!live) return; if (isGrantRefusal(e)) gone(errText(e)); else fail(errText(e)); });
  const id = setTimeout(() => { if (live) again(); }, everyMs);
  return () => { live = false; clearTimeout(id); };
}

/**
 * Follow a `Watch` stream: apply each delta to the rows (by `key`) and hand the rows on. The cell ends every stream
 * after its max age; then `again` opens a new connection, so an ended or revoked grant is refused within that age and
 * the panel leaves the page.
 */
export function watch<R>(open: () => Feed<Delta<R>>, key: (r: R) => string, set: (rows: readonly R[]) => void, fail: (why: string) => void, gone: (why: string) => void, again: () => void): () => void {
  let live = true;
  let feed: Feed<Delta<R>> | null = null;
  const rows = new Map<string, R>();
  const loop = async (): Promise<void> => {
    try {
      feed = open();
      for await (const d of feed) {
        if (!live) return;
        if (d.reset) rows.clear();
        for (const k of d.del) rows.delete(k);
        for (const r of d.put) rows.set(key(r), r);
        set(Array.from(rows.values()));
      }
    } catch (e) {
      if (!live) return;
      if (isGrantRefusal(e)) return gone(errText(e));
      fail(errText(e));
      await new Promise((r) => setTimeout(r, 2000));
    }
    if (live) again();
  };
  void loop();
  return () => { live = false; feed?.close(); };
}
