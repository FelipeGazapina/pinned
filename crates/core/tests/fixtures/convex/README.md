Requests and responses captured from a local Convex backend (`precompiled-2026-10-22`, npm `convex`
1.49.0) on 2026-11-11, with the admin key replaced by `<admin-key>`. `auth` names what the request
sent: `admin` the admin key in `Authorization: Convex <key>`, `none` no header, `wrong-key` another
key.

`url_for_key.json` and `team_and_project_for_key.json` were captured from `api.convex.dev` with a real
production deploy key on 2026-11-11, with the key replaced by `<deploy-key>` and the names by
`quick-lemur-789`, `acme` and `shop`. `bearer` names the `Authorization: Bearer <key>` header they
were sent with.
