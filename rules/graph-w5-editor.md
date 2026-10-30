# U5 graph workflow editor (ui)

Branch `graph/w5-editor` from `workflow-graph/base` (6ccdef6).

## Delivered

- `ui/graph.js` — pure graph helpers: `ports`, `splice`, `append`, `connect`, `disconnect`, `remove`, `move`, `validate`, `newNode`, `ensureGraph`, `defaultGraph`.
- `ui/graph.test.js` — `node --test ui/graph.test.js` (5 tests).
- `ui/settings.js` — canvas editor over `connector.graph`; session panel via talk node; save sends `graph`.
- `ui/graph-harness.html` + Playwright script at `.audit/n8n-graph/swarm/w5-graph-editor-playwright.mjs`; screenshots under `.audit/n8n-graph/swarm/w5-screens/`.

## Test commands

```bash
node --test ui/graph.test.js
cd /tmp/pw-graph-w5 && node /Users/felipe/projetos/personal-harness/.audit/n8n-graph/swarm/w5-graph-editor-playwright.mjs
```

## STEPS not fully ported to node panels

- **reply**, **bug**, **coding**, **review**, **proof** — not graph node kinds (session callbacks / connector fields); no per-node panels.
- **ask** optional step / **stopOn** — classify has no `stopOn`; old ask stop rules not modeled on graph nodes.
- **decide** trigger routing UI (categories → mention/workflow, fallback thresholds as category names) — replaced by classify + edges; no “route panel” or inline “new workflow for category”.
- **picked** / **coding** / **review** column optional steps — only **stage-picked** node; column names still on connector `columns`, not separate move nodes for coding/review.
- **Trigger kind switcher** (`kindHtml` / reshape) — not on trigger node panel; watch kind fixed when graph is created.
- **Ghost optional steps** (+ ask / + proof menu at chain end) — use dangling-port / edge + picker instead; no legacy ghost tiles.
