Requests and responses captured from a local Convex backend (`precompiled-2026-10-22`, npm `convex`
1.49.0) on 2026-11-11, with the admin key replaced by `<admin-key>`. `auth` names what the request
sent: `admin` the admin key in `Authorization: Convex <key>`, `none` no header, `wrong-key` another
key.

`url_for_key.json` and `team_and_project_for_key.json` are synthetic until a real deploy key captures
them. Their routes, body and `Authorization: Bearer <key>` header follow the source of npm `convex`
1.49.0 (`src/cli/lib/deploymentSelection.ts` near line 728, `src/cli/lib/api.ts` near line 978), which
reads the first answer as the deployment's URL and the second as slugs. `bearer` names that header.
