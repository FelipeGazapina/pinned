# Changelog

## 0.18.1 (Unreleased)

### Fixed

- Finished workers no longer look active. Under a Captain in the sidebar, the workers whose turn ended now fold into a "Finished" group that opens with a click, and the workers still running, waiting on you or failed stay where they were. A Captain's Workers card stays in the turn that started its workers, and no longer follows the conversation to its end once every worker is done, and when it lists only the workers at work its header still counts every worker. The global Orchestrator's `projects()` count no longer includes workers that already finished, and the Projects page counts workers the same way.
- Slack: a mention in a thread that a workflow already follows now files a ticket when it asks for one, and a second request in the same thread answers with the same ticket. Already on main (#193).

### Added

- Upstash Box: a default box size for each account and each project, set under Delegation limits on the Way of working page. Already on main (#195).
- Slack: a triage session can set the ticket's title and summary before the workflow files it, with `pinned run set` and `{{vars.*}}` in the workflow's ticket fields. Already on main (#196).
- Usage page: an Upstash Box section with what Upstash reports for this month's boxes, pinned's own count of box hours, and the boxes running now. Already on main (#199).
