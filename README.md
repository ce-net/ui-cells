# ui-cells

The minimum shared pieces a full-stack cell needs to put its face on a page, built for the control room's slice 1
(repo:control-room docs/DESIGN.md section 13) and usable by any site. Apache-2.0.

| Piece | What it does |
|---|---|
| `crates/ui-part` | `views!`: the `ui { }` half of a cell contract (props, needs, events, data, streams, slots, speaks) and `parts` (field-level rights). `cargo build` fails when a view needs less than its data ops need or names an op the api does not have. `Parts::withhold` blanks a field for a caller without its right. |
| `crates/ui-chain` | The caller's verified chain in a view cell: rights, chain id, the grant a cell presents to a source, and `layer_view`, the row filter for `where` (equality and set) caveats. |
| `crates/ui-manifest` | ui.manifest `Resolve`: a site's route and slot tree, per viewer, to only the views their rights may receive; ui.serve's `may_fetch`. |
| `crates/site-gateway` | The minimum site.gateway behind the apps.ce-net.com sign-in: the person's grant, their manifest, only their bundles (by sha256), one socket to wire.ws-door. |
| `crates/ui-gen` | `ce-wire-gen --lang ts-ui` as its own tool: the `views!` block to typed TypeScript bindings over the ts-web client. |
| `ts/ui-rt.ts` | `defineView`, the view runtime: data from the cell's own ops, `Watch` deltas, reconnect after a stream's max age, `Part` and `show`. |
| `ts/ui-host.ts` | Boots a page: manifest, digest check of every module before it runs, slots, panels leave when a grant ends. |
| `ts/kit` | Tokens, components and the element builder every view draws with (light theme by default). |

Not yet: `ui { }` inside `ce_wire::api!` itself, a signed manifest, `decl.lint`, ui.manifest and ui.serve as their
own cells (today they are libraries the gateway links).
