# Proof: New workflow › Linear no longer redirects to Connectors (PR #222)

Recorded against a local hub (throwaway Postgres, this branch's `ui/`), invented data, fake keys only;
the hub's outbound HTTPS went to a dead proxy, so no Linear or Slack service was contacted.

- `workflow-linear-redirect.webm` (≈37 s): Linear not connected → the option says why, Connectors explains
  which workflow sent you there → Linear connected (fixture) → Back to the workflow opens the Linear editor;
  then with Linear connected, New workflow › Linear goes straight into setup. Steps: `steps.md`.
- `before-v0.18.1-linear-connected.png`: v0.18.1 UI on the same hub with Linear connected — the option
  reads "Connect Linear under Connectors first." and the click lands on Connectors.
- `after-linear-missing-note.png`: the fix, Linear missing — the note on Connectors.
