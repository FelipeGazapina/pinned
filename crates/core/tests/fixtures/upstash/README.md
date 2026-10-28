Shaped from `@upstash/box`'s `BoxData` and `ExecResult` types. Replace them with responses captured from the real API
(`GET /v2/box?label=…`, `POST /v2/box`, `GET /v2/box/{id}`, `POST /v2/box/{id}/exec`), with ids and labels kept, when one is at hand.
