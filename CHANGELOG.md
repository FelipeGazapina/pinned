# Changelog

## 0.18.10 (2026-11-23)

### Added

- Settings › Way of working has "Boxes at once": how many Upstash boxes the account's agents may have open at once. It covers boxes the Orchestrator, Captains, workers and workflows open. It defaults to 8 and runs from 0 up to the hub's cap. Boxes you open yourself never count. At 0, agents cannot start a box on the account.
- A Captain's plans open in the right panel (key N). Each step says who runs it, and the glance names the step that waits for you first. On the Orchestrator, the panel is "Captains": one card per Captain with its plans, state word and progress. The rail uses the same word as the card.
- The Plan ready card has "Approve with note": it approves the plan and sends your note as the next message in one step, on the desktop and the phone.
- A session can name itself with `set_session_title`. Its own name shows in the sidebar over the generated title. A title you set yourself still wins.
- The Model prices dialog lists the models that ran with no price yet, each with "Set price".
- A team run's model cost now counts toward its bot's spend limit. The run page shows "Box US$ x · Model US$ y".

### Changed

- A worker under a budget does not start on a Claude model pinned has no price for. It says so and points to Settings › Usage › Model prices. Cursor and Codex workers are held by their time limit as before.
- OpenCode Go offers only the efforts its CLI takes. A failed model-list refresh waits a minute before it asks again.
- Settings › Workflows reads the settings again once a minute while it is open, and its "Last checked" line shows its age when the check is older than 3 minutes.

### Fixed

- The worker strip, the phone's worker count and the projects page no longer show a finished or sleeping worker as working with a running clock.
- An approved plan whose text says "Interrupted" no longer reads as a refusal and no longer leaves an Approval on Needs you.
- Every hub hello rebuilds the confirmations waiting on you, so one lost to a dropped socket shows again.
- Rooms: a post to a seat that has no tab yet waits for it, a settled room owes nothing after a restart, closing a tab ends the room it owns or the seat it holds, and a repeated `room_post` id appends nothing.
- A session stopped on the docs-server dialog names the folder as [folder] in its failure, never the raw path.

### Known gaps

- A titled request sent to a Captain on another machine or in a box arrives without its title.
- Copying an account's setup does not copy Boxes at once yet.

## 0.18.9 (2026-11-20)

### Added

- A project's Captain can open a room with `room_open`: one shared thread several workers sit in. Speech, the brief and each seat's summary live in the room. Each seat keeps its own tools, diffs and worktree. A line you type in the room wakes every seat still working, and the Captain. When every seat finishes, the room closes.
- The Workflows page now offers "A Linear issue changes status" and "A Linear issue is assigned". They use the Linear key already connected. The hub already fired these; the picker, the editor and the graph can now make them.
- OpenCode Go's model list is fetched live from the gateway. A model the gateway adds shows up without a pinned release. Free models get a green Free badge in the picker.

### Fixed

- A Slack workflow that posts its acknowledgement and then cannot open the session stays on that session step. The next check retries the open. A later message in the thread that does not mention the bot does not start another run. A failed box boot still gets one retry on that same run. The thread stays claimed. A second boot failure within 30 days stops the run.
- Filing a Linear issue from a Slack thread asks the decision model over every open issue's title in that team, and answers the one that already covers the report instead of filing a copy.
- A session that stops on Claude Code's "Use the docs server this folder lists?" dialog now ends its start with that reason, instead of waiting out the 10-minute limit. pinned never answers that dialog for you.
- Hub checks keep live workflow runs plus the newest 200 finished ones, instead of every run ever started, and name the offline-runner hold as held in the logs.

## 0.18.8 (2026-11-19)

### Added

- This Mac keeps working when the hub is stopped or unreachable. Threads on this Mac stay live. Threads on other machines dim and freeze where the link dropped. The bar reads "Can't reach the hub. This Mac keeps working." This Mac counts as online. Clone and update run here. Connectors and workflows show the last saved copy and stay read-only. A tool that can answer on this Mac does. The others say the hub can't be reached.
- After the hub has been unreachable for five minutes, the Mac named as the offline runner checks Linear and Slack and runs the account's workflows from a copy it kept while the hub was up. Settings, MCP servers and tastes on that Mac come from the copy. Another desktop keeps its own sessions, and workflows stay on the runner. When the hub returns, the runner sends those edits back. The hub holds workflows until that handback, or until you choose "Run them on the hub now". Signing out while this Mac still has edits to send back asks first. Confirming drops them.

### Changed

- Sync fetches a project when the hub says it changed, and only the files written since the revision this Mac already has. A machine also checks ten minutes after its last read, and reads the whole list once a day. A hub older than 0.18.8 still answers the full list every 20 seconds.

## 0.18.7 (2026-11-18)

### Added

- Slack: a workflow that sorts a channel's messages can now also read posts from apps and integrations, such as alerts, CI runs, deploys and pull request notices. Tick "Also read posts from apps and integrations" in its trigger; it is off by default. pinned's own bots are never read. A check reads at most 10 app posts per workflow, and app posts past those 10 are skipped. The text of an app post reaches the model and the session marked as untrusted data.
- The global Orchestrator now puts the questions waiting on you, its own and its Captains', on one page named "Pendências", with options to tick and a note field, and links that page in its reply. It publishes the page as a claude.ai Artifact at the same link each time and reads your answers back from it.

### Changed

- Sync: the hub no longer limits each account to 1024 MB. An account whose sync stopped at that limit syncs again. The hub now has one limit for all accounts together; when it is full, a push that would grow an account is refused, and nothing is deleted.
- Captains and workers no longer answer with only "ok". They reply once the answer is final, say what they did or found, and name a task or a file instead of pasting its contents. A Captain asks its most blocking question first, treats a question you skip as skipped for that task only, and proposes a test or a repo rule when you make the same correction a second time.
- A worker no longer gives `rm` a path built from `$(…)` or backticks. It runs the command first, then removes the literal paths it prints.

### Removed

- Convex: the `convex_query` tool is off for every session. It ran any JavaScript against a project's Convex deployment with its admin key, which can read the deployment's environment values. `convex_tables`, `convex_data`, `convex_run` and `convex_logs` are unchanged. An older app that still offers `convex_query` gets a refusal that points to `convex_data`.

### Fixed

- Hub web UI on a phone: while a thread or the Orchestrator is open, the page no longer redraws the hidden thread list or animates its status icons. With a busy Orchestrator on a simulated slow phone (6x CPU throttle), the page is busy 34% of the time, down from 59%, and drops 11% of scroll frames, down from 23%. The slowest keystroke takes 40 ms, down from 80 ms. Opening a thread takes about as long as before.
- Convex: the Tables hint in a workflow's connection no longer says a run can query. It now says that All tables also lets the run read logs, and that a table list turns logs off.
- pinned no longer types into a session while it shows a menu or dialog of its own, such as a permission question. Queued messages, task notices, pull request updates, the continue after a usage limit and a skills reload wait until it closes. Before, the keystrokes could reach the dialog and the message was lost.
- A task notice is no longer lost when pinned stops while it delivers it. A notice that was picked up but not typed is pending again the next time pinned starts, so after a crash it can show twice but never zero times.
- Pull request updates: a new comment no longer replaces an older one the session has not heard yet. The session hears every unheard comment, oldest first. When more than 10 wait, it hears the newest 10 and a line saying how many earlier comments are not shown.
- A message sent to a busy Captain is no longer lost when the Captain's end-of-turn hook stopped waiting before the reply reached it. The message stays queued and is typed into the session at its next idle pass.
- Captain queue: choosing a model in the `/model` picker no longer holds queued messages until you type. `/statusline` and a line that starts with a space now start a turn, as Claude Code reads them, so queued messages wait for that turn to end.
- Threads show Compacting for every compaction, including the automatic ones, not only after a typed `/compact`. A Compacting mark that never ends clears after 10 minutes.
- A turn that ended on an API error now shows as "Stopped on error" in Needs you, with Try again and Dismiss, instead of as a question waiting for your answer. The card shows the error, cut at 200 characters. Dismiss hides it only on that device.
- Shared history: two machines that sync it at the same moment both keep their new lines. Before, one machine's push could replace the other's lines on the hub.
- A Claude Code session that pinned starts now carries the project MCP servers you already approved in the main checkout, so a session in a pinned worktree, or under another of your Claude accounts, no longer asks again. A server whose definition differs from the one you approved is not carried.
- A session that stops on Claude Code's "New MCP server found in this project" dialog now ends its start after 60 seconds with the reason "Claude Code is waiting for approval of this project's MCP servers in <folder>; open the session to answer it". Before, it stayed in starting for 10 minutes with no reason. pinned never answers that dialog for you.
- A read-only worker no longer stops at the plan-approval menu, where nobody answers it. It starts with writes refused instead of in plan mode, and a restore, a resume or a hand-off to a tab never runs it with permission prompts skipped.
- Upstash Box: a worker stuck on a startup dialog before its first prompt now ends after 10 minutes with that reason, instead of running until its time limit or budget. If its machine does not end it, the hub ends it at 12 minutes.
- Upstash Box: a worker box is never taken as the global Orchestrator's host. Before, a box that answered as host made messages to Captains fail, or counted as the host when none of your machines hosted.
- Upstash Box: an agent's request to extend a box is now checked against the box and the Box budget when the agent asks, not only after you approve it. A request that can never extend anything is refused at once. At most 3 requests per box and 10 per account wait for you. The agent gets an answer, or "busy", within about 2 seconds. When a box ends, its waiting requests close as denied, and agent activity shows them as denied.
- Teams: when the hub cannot read one bot's settings, that bot's waiting run keeps its place in the line instead of being dropped. After five failed reads it is dropped and counted as a failed start. Other bots' runs go on meanwhile. This was a known limit of 0.18.6.
- Teams: the same event never starts a bot twice, and the database refuses a second live run for one bot.
- Teams: when the hub cannot read the team's workflows, keys or settings, the owner sees on the team's page and in the team nav that the team's line is held, since when and why.
- Teams: a run that has ended answers "this run's log was not kept after it ended" when you ask for its log, instead of "the team has no such run".
- Teams: the bot wizard no longer offers "A Linear issue gets a label". A team keeps no Linear key, so such a bot could never start. A bot made earlier with that trigger still shows it on its page.
- Teams: the owner can connect the team's AI account at any time with "Connect AI account" in the team nav or on a bot's page. Before, only the create-a-team wizard offered it, and the bot page pointed to a settings page that does not exist. A member's bot page now reads "Ask <owner> to connect the team's AI account."
- Teams: switching to a team opens the team's home. While a team is current, personal places say so: "New personal thread...", "Personal · Threads" in ⌘K, "Needs you · Personal" and "Personal · " before a personal thread's prompt.
- Teams: a bot made in the wizard shows in the team sidebar at once, without a reload.
- Teams: the wizard's Done row has no step number, so the list matches "Step n of 4", and the invite code field shows the code's real shape, `ABCD-EFGH-JKMN-PQRS`.
- Needs you: an agent's request card says "Asked by …" once, not twice.
- The sidebar keeps your project filter on a fresh page load. Before, a hub message that arrived early reset it to All projects.
- Sync: when the hub refuses a push because a limit is reached, the machine waits before it tries again, from 20 seconds up to 10 minutes. While it waits, it still sends changes that do not grow a project, and it pulls nothing over a local change it has not sent. The sidebar shows "Sync stopped on <machine>" with a Review button until the hub takes a push again. The refusal says that nothing is deleted and no longer tells you to delete old sessions.
- My reports: a report already marked fixed moves to the version in a `fixed-in:` label added to its issue later.
- Upstash Box: a box can now only read the account's files. The hub refuses every change a box sends that the account's other machines or apps would read, such as synced files, workflows, MCP servers, Way of working or a new folder on your machine, also when an agent in the box asks. A box can still hand a task to another box and send its report.
- Workflows: staging a Linear issue, replying in a Slack thread, and filing or taking a Linear issue through a workflow's connections are now accepted only from the session that runs that workflow on that issue or thread, while it runs. Another session gets "only the session that runs this workflow can do that".
- Only you can change "Review each diff before accepting" and "Second opinion on risky changes", on the Way of working page. An agent can read them but never change them, with or without your approval. A request to change them that was waiting for you ends with nothing changed when you approve it.
- An agent that writes Claude Code's own settings or hooks in `~/.claude` (`settings.json`, `settings.local.json`, `hooks/`), or the project's MCP approval files from another checkout, now counts as changing its own permissions, which a schedule can never pre-approve.
- A session pinned starts no longer inherits the role, project, tab, account, box or Claude home of the session or machine that started it.
- A login-account session no longer inherits `ANTHROPIC_*` settings that point at the OpenCode Go gateway.
- If you export `CLAUDE_CONFIG_DIR` for the built-in account, plugin sync now shares that home instead of the default one.

### Known limits

- Workflows can start when a Linear issue's status or assignee changes, once per change, but the workflow editor cannot make such a workflow yet. Only an agent can, with `save_connector`. The Workflows page shows that workflow as "Open this workflow to repair its trigger", and its Run now is off. A workflow starts at most once every 15 minutes for one issue, an issue starts at most 4 runs an hour, and a change pinned made itself does not start one. Team bots don't use these triggers.
- A workflow trigger on a schedule can be saved but does not start the workflow yet. To run a workflow on a schedule, make a scheduled task whose action is that workflow.
- A Captain now keeps a plan for each request that takes more than one step, and links its delegated tasks to the plan's steps. The app does not show these plans yet.
- Apps older than 0.18.7 do not wait after a refused push, and they show the old "would pass 1024 MB" text when the hub is full.
- Your own Extend of a box in the app does not wait its turn. If it races your approval of an agent's request for the same box, one of the two increments can be lost. A box never ends above its ceiling.
- After `/model`, Esc, Up and Enter, the prompt recalled from history still counts as a local command, so the Captain queue can type a queued message after 10 seconds while that turn runs.

## 0.18.6 (2026-11-16)

### Added

- Teams: create a team, invite people with a code, and attach projects to it. A team keeps its own keys (Upstash Box, a Claude token for boxes, GitHub, Slack) on the team, never on a member's account. A member can leave, and the owner can remove members or take a project out of the team.
- Team bots: a bot is a team workflow that runs in a fresh Upstash Box under the team. A new or edited bot is a draft until a member of a group it calls, or the team owner, turns it on. A bot starts with Run now, a Linear label or a Slack message. It runs one at a time; later starts wait in line. Each bot has a time limit, at most 4 hours.
- Calls: when a bot is unsure, or before an action that can't be undone, it asks a group of people instead of guessing. The first answer wins. A call nobody answers goes to the team owner and then gives up, which ends that run. Time spent waiting on a call does not count toward the bot's time limit, and the run shows waiting time apart from active time. A call can ping a Slack user group through the team's Slack bot.
- Report an issue: from the app, or from an agent after you confirm, file an issue on pinned's GitHub. Paths, names, e-mail addresses and secrets are removed before it leaves, and you see exactly what will be sent. My reports lists your reports and their state; a report that matches an existing issue adds your case to it.

### Fixed

- Chat: a message sent while the session is busy now shows as queued and is not sent twice. If the connection drops before delivery is confirmed, the message stays with a note to check the session before sending again.
- Sync: an account at its sync size limit no longer retries a refused push every second. A push that does not grow the account is accepted, and deletions are sent first.
- Skills: long location chips and the editor header no longer overflow their panes at narrow widths.
- An override of a provider address is now limited to development builds.

### Known limits

- Spend limits for bots arrive with model cost reporting. Until then a bot's limit is its time; the box budget still applies.
- Bots don't run on a schedule yet, and GitHub events don't start bots.
- Team bots: if one read of a bot's settings fails, for example while the database is briefly unreachable, that bot's waiting run or event can be dropped and counted as "start failed" instead of waiting for the next pass.
- A team box can end if two database writes fail in a row while its run opens.
- A bot whose classify step answers by quoting its whole input refuses the run instead of going on.
- A box whose node is older than 0.18.6 skips a step that asks the team while the hub is briefly busy, instead of waiting and trying it again.
- Issue reporting is for personal accounts in this version; a team cannot file reports.
- The limit on joining a team with wrong invite codes is kept in the hub's memory and resets when the hub restarts.

## 0.18.5 (2026-11-15)

### Fixed

- The hub now returns freed memory to the system. Its Linux build used the allocator of its static build, which kept the memory of the many small allocations each machine's sync reply frees. On a local hub under load shaped like production's, resident memory grew about 1.5 GB an hour while the memory actually in use stayed under 31 MB. With the new allocator (mimalloc) it stayed between 67 and 110 MB over a 21-minute run. Longer runs and the x86_64 build under load have not been measured yet.
- `pinned-server` on Linux machines, which is also the node, uses the same allocator.

## 0.18.4 (2026-11-14)

### Fixed

- Workflows: a run waiting on an agent tab no longer repeats its step when that tab finishes its turn and closes in quick succession.
- The hub's saves of workflows and connector keys to its database no longer queue without limit. Each account keeps only its newest unsaved copy, an older copy no longer replaces a newer one from the same hub, and a stopping hub writes what is still waiting first. During a deploy, the stopping hub can still overwrite a save from the new hub once; the new hub's next save restores it.
- The hub's database connection now has time limits: connecting, each statement and each write give up instead of waiting forever, and a stuck connection is replaced.

### Changed

- The hub's periodic census log now reports heap memory in use, bytes queued per connection, traffic by message type, sync bytes and per-account sizes (counts and sizes only, no content). The hub's memory growth is not fixed in this release: these measurements are there to find its cause, and the bounded saves above remove one possible cause. Counting heap memory costs about 5% of the hub's CPU.
- The hub's accounts table gains two version columns, added automatically when the hub starts. An older hub keeps working with them.

## 0.18.3 (2026-11-13)

### Fixed

- The hub's memory no longer grows while machines send frequent tab updates: tab updates that arrive while workflows are being driven are merged into one pending update per account instead of queuing without limit.
- Calls to Slack, Linear and GitHub now time out after 30 seconds, so a stalled call cannot hold up workflows.

## 0.18.2 (2026-11-13)

### Added

- Ideas: the Orchestrator and your sessions can now save things you might do someday with `save_idea`, list them with `list_ideas` and delete them with `delete_idea`. Ideas are kept on your account, apart from memory, and are never fed into a prompt on their own. The Ideas page comes in a later update.
- A thread that stopped on an API error while the computer slept now shows a Needs you card saying when it slept, with a Continue button (for a Captain, also how many delegated tasks are still running). Nothing is typed for you.

### Removed

- Workflows: Generate with AI is gone. Ask an agent, such as the Orchestrator, to create a workflow from a description; older apps that still press the button get a short message saying so.

### Fixed

- Workflows: a session step's permission is read the same way when it is saved, checked and run; a value that is not one of the exact names is refused at save and opens no session. The hub no longer accepts from the app the calls only it sends to a machine.
- Workflows: a workflow that holds a step whose permission pinned cannot read (saved by an older version) now starts no run when its trigger fires, and every check shows an error naming the workflow and the step until you fix it. Before, the run took the Linear issue or Slack thread and stopped, and the error showed only until the next check.
- Workflows: an agent session that asks for permission prompts can no longer change when or where a stored workflow session that skips them runs, also through a workflow it calls. A workflow or step model that starts with `-` is refused at save. The hub now reads an agent session in plan mode, or in a permission mode its machine does not know yet, as one that asks for prompts, as the machine itself does. This also covers a workflow saved without a graph, such as a Slack router whose categories call other workflows.
- Workflows: moving a signed-out app's workflows to its account now runs every workflow through the same checks a save does (its graph, each step's permission, its model, Call cycles and its Convex step). One that fails stops the whole move, naming the workflow and the check, and nothing is written.
- Workflows: moving a signed-out app's workflows to its account no longer replaces what the account set up itself: its Upstash account, box permissions, box answers, repo insight, box Claude token hints and Slack thread claims stay as they were, as its MCP servers and Convex deployments already did.
- Workflows: moving a signed-out app's workflows to its account is now refused when the account has only a Convex deployment connected, as for any account that is not empty. When the move is refused for a workflow, the message shows its workflow and step names on one line, each cut at 80 characters.
- Pressing Shift+Tab in a session now tells the hub at once that the session's mode is unknown, so its agents are treated as read-only from that moment rather than after the next tab update.
- Upstash Box: once pinned has failed to delete a box, the Usage page says so and why, also when Upstash is not connected, and the Usage entry in the sidebar is marked. For a box whose Upstash id pinned has, the notice says it is still running, and may still be billed, on Upstash and can be deleted in the Upstash console, and lists its Upstash id. For a row that never recorded a box id, pinned cannot tell whether a box exists, so the notice says that, lists the label pinned would find it by, and asks you to check the Upstash console. When Upstash is not connected, the notice says reconnecting the Upstash key lets pinned try again on its next pass. The mark comes from pinned's own record and asks Upstash nothing; a box pinned is deleting as usual is not listed. Before this, only the server log said anything.
- A worker box the hub has ended but could not delete now stops its Claude sessions and its node once the hub has said for 15 minutes that it does not know the box's machine. The box itself stays on Upstash, and still costs, until it is deleted. A "One box, reused" box does not do this and keeps its sessions. A box started before this release keeps the old behaviour until it boots again. A hub that is slow to read a machine's sign in now asks it to try again later instead of refusing it, so a slow or busy hub never counts as a refusal.
- Workflows: in New workflow, clicking Linear (or any Slack option) with that connector connected now continues the setup instead of opening Connectors. Since 0.17.10 every option sent you to Connectors and said "Connect Linear", whatever was connected. When the connector is missing, Connectors now says which workflow sent you there, and connecting it, or Back to the workflow, returns you to that workflow as you left it.
- A Captain no longer keeps the global Orchestrator's messages queued for hours after its turn ends. When a turn ended just as a message came in, the Captain's end-of-turn hook could give up waiting for pinned before pinned handed the message over. The turn ended without it, while pinned kept counting the Captain as busy, so every later message waited until you typed into the Captain. The hook now waits long enough for the hand-over. If a hand-over still gets lost, the queued messages start a new turn once Claude Code reports the Captain idle, about a minute later.
- Stopping a delegated agent now refuses to signal pid 0, pid 1, every process (-1), pinned itself or its own process group, so a bad pid among the agent's processes can no longer reach pinned or every process on the machine.
- Agent tabs no longer read working or starting with nothing running behind them: a session killed mid-turn now reads stopped with its reason (its time limit, stopped by you, or its exit code), sleeping and resumed tabs read idle, a start that never reports fails after 10 min, and the Orchestrator's `projects()` count skips workers with no live process.
- A worker box whose link to the hub drops for a moment keeps its task. Before, a box running for more than 5 minutes that lost its link was deleted at once as "the box did not connect within 5 min". Now its task waits for it to come back, and a box that stays offline for 10 minutes ends with that reason.
- A worker's report is no longer lost when its Captain's machine is asleep or offline. The hub keeps the report and hands it over when the machine comes back, and the worker's box still ends at once. A report the machine no longer knows the task for, as after a restart, is kept for 30 days and listed by `unclaimed_ends`.
- Upstash Box: removing a box machine, disconnecting Upstash or ending a worker now deletes its box with only the Upstash key stored, and one box that will not delete no longer stops the sweep from deleting the others. A box that reaches one of its limits (time, idle, budget) is now deleted too when only the key is stored.
- Upstash Box: only the "One box, reused" box is now created to keep running while idle, which Upstash bills at a fixed monthly price per box. A worker's box, a Captain's box and every other temporary box is now a standard box: Upstash pauses it when it is idle and bills it for the CPU time it uses. Boxes made before this release keep the setting they were made with.
- Upstash Box: the cost shown for a box, and its budget, now use the price of its size: $0.10 an hour for Small, $0.20 for Medium, $0.40 for Large, where every box was counted at $0.10. A box's budget must now cover at least an hour on its size (the message names the figure), and a budget runs out sooner on a bigger box. These figures are a ceiling: Upstash bills a standard box for the CPU time it uses, not for every hour it exists, and storage ($0.10 per GB-month) is not counted.
- Upstash Box: a Medium or Large box already running when the hub updates to this release has less budget left at once, because the new price applies to its whole age, not only to the time after the update. Such a box can reach its budget and end sooner than it would have.
- Workflows on Upstash Box: a workflow run whose box ends before the run does now fails, and the workflows card says why: the box went offline for 10 minutes (as when Upstash pauses an idle box), reached its 4 hour limit or its budget, its machine was removed, Upstash was disconnected, or the box was deleted outside pinned. Before, the run waited on a box that was gone until the box's 4 hour limit, and kept its issue or thread. A box that reached its budget or its 4 hour limit, or whose machine you removed, or whose Upstash you disconnected, keeps the issue or thread handled: the run starts again only when you run it. A box that failed to boot or went away otherwise (offline, deleted outside pinned) frees the issue or thread once per issue or thread per 30 days, so the workflow's next trigger may start it one more time; if a box ends the same way again within 30 days, the issue or thread stays handled until you run it.
- Upstash Box: an app or an agent can no longer name which box is the "One box, reused" box when it saves the box settings. Only the hub picks it; a save keeps the box the hub picked, and an older app that sends back the box it read keeps working.
- A project's model ceiling (Tier) no longer lets through a Claude model id that this machine's `claude` does not list, nor an empty model name, because the words of an id are no proof of its cost. A remote worker's model is checked against the Claude list of the machine that runs it once that machine is on 0.18.2; until then the sending machine refuses a Claude id it does not list.
- Extending a box now asks for your password when pinned is locked.
- An agent that asks to add money to a box now waits for your approval.

## 0.18.1 (2026-11-12)

### Fixed

- Finished workers no longer look active. Under a Captain in the sidebar, the workers whose turn ended now fold into a "Finished" group that opens with a click, and the workers still running, waiting on you or failed stay where they were. A Captain's Workers card stays in the turn that started its workers, and no longer follows the conversation to its end once every worker is done, and when it lists only the workers at work its header still counts every worker. The global Orchestrator's `projects()` count no longer includes workers that already finished, and the Projects page counts workers the same way.
- Slack: a mention in a thread that a workflow already follows now files a ticket when it asks for one, and a second request in the same thread answers with the same ticket. Already on main (#193).
- Dictation on macOS: clicking Allow no longer ends in "Microphone access denied". The app now carries the microphone entitlement that macOS requires before it will ask. While Microphone or Speech Recognition is off, every click on the mic says which one and opens its System Settings pane; in a browser it says how to unblock the microphone for the site. macOS may ask again for Microphone and Speech Recognition after an update, because the app is not yet signed with an Apple Developer ID.
- Settings: an action that fails now says why in a note at the bottom of the window. Before, a failed click, Enter in a token field or ⌘S showed nothing, so a refused Disconnect looked like a click that did nothing.
- Windows desktop app: MCP server values (environment variables and headers) could not be saved, and the Upstash Box Claude token was kept in a hidden NTFS stream that copies, zips and backups left out. Both are now stored under an escaped file name, and a token kept the old way is still read.

### Added

- Linux desktop beta: an AppImage and a .deb for x86_64 and arm64, next to the .dmg and the Windows setup .exe, with updater entries in `latest.json`. The AppImage needs FUSE, or run it with `APPIMAGE_EXTRACT_AND_RUN=1`. Dictation and browser-panel screenshots need macOS and are not available on Linux. Not tested yet on Linux: the tray icon on a real desktop, including GNOME, Wayland (it runs through XWayland), notifications, keep awake, and the updater end to end. Beta testers, please report problems with those. The `pinned-server-*-linux-musl` files on the same release are the headless node, not the desktop app.
- Upstash Box: a default box size for each account and each project, set under Delegation limits on the Way of working page. Already on main (#195).
- Slack: a triage session can set the ticket's title and summary before the workflow files it, with `pinned run set` and `{{vars.*}}` in the workflow's ticket fields. Already on main (#196).
- Usage page: an Upstash Box section with what Upstash reports for this month's boxes, pinned's own count of box hours, and the boxes running now. Already on main (#199).
- Sessions that run in an Upstash Box show the Upstash mark beside their name: in the sidebar (threads, Captains and workers), the Workers card, the phone Workers sheet and Needs you. The mark comes from the machine's Upstash Box field, not its name, and reads "Running in an Upstash Box" to screen readers.
- Upstash Box: a box pinned opens is named after its project and role, e.g. `pinned-worker-7fc0a1`, in the Upstash dashboard. Boxes already running keep their old `pinned-<id>` names until they end.
- Slack: connect several bots, one per workspace or Slack app. The Slack card lists them by workspace and shows which workflows use each. A workflow picks its bot in the trigger, next to the channel. Its replies, tickets and the workflows it calls use that bot, and a thread stays with the bot that first answered it. An existing Slack connection becomes the first bot, the default, with nothing to redo. Video: [connecting a second bot](https://github.com/FelipeGazapina/pinned/releases/download/v0.18.1/multi-slack-ui.webm).

### Changed

- Opening a long thread, on desktop and phone, now shows its newest 300 messages first and reads older ones as you scroll up. On a 2,000-turn thread the last turn showed in 0.65 s instead of 2.8 s. A machine still on 0.18.0 sends the whole thread as before, so update it too.
- Disconnecting Slack is now refused while any workflow uses that bot, paused workflows included. This also applies with a single bot and from an older app. Move those workflows to another bot, or delete them, then disconnect. The message names the workflows.
- On a 0.18.0 desktop app, Disconnect for Slack appears to do nothing while a workflow uses the bot. Update the app, or move or delete those workflows first.

### Rolling back to 0.18.0

- Slack: once a hub, or a desktop app used without signing in, is back on 0.18.0 and saves its settings, only the first Slack bot connected is kept. Workflows that used another bot then run with that first bot. After updating again, connect the other bots and pick them in those workflows again.
- If you roll the desktop app back to 0.18.0, enter again any MCP server value or Upstash Box Claude token you changed while on 0.18.1. 0.18.1 saves those keys under a new file name, and 0.18.0 doesn't find that file. Keys 0.18.1 only read still work after a rollback. Slack, Linear, GitHub, Vercel and Upstash keys keep their names.
