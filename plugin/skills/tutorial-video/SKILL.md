---
name: tutorial-video
description: Record a feature demo or proof-of-work video as a tutorial a person can follow, by navigating the site through visible clicks on its sidebar and nav links, never by jumping to URLs. Use for every video that presents or proves a feature, including the Playwright proof recorded before `pinned evidence`.
---

# Tutorial video

A viewer must learn where each page lives. The recording clicks the sidebar, the top nav and the tabs the way a person would. It never types a URL, calls `page.goto` after the first page, reloads, or goes back.

## Record

1. Find how a person reaches the feature from the app's first page. Read the sidebar, nav bar and tabs in the running app or its code. The path is the script.
2. Write the script in a temporary directory outside the repository. Load Playwright from the project (`createRequire('<repo>/')`) or run `npm i -D playwright && npx playwright install chromium` there.
3. Drive it with the helper at `$PINNED_TUTORIAL_HELPER`, which pinned sets in every session. It owns the cursor, the ring on each target, the caption and the rule against jumping.

```js
import { createRequire } from "node:module";
const { chromium } = createRequire("<repo>/package.json")("playwright");
const { startTutorial } = await import(`file://${process.env.PINNED_TUTORIAL_HELPER}`);

const t = await startTutorial({ chromium, dir: "<tmp>/video" });
await t.open("http://localhost:3000/", "Where we start");
await t.click(t.page.locator("aside").getByRole("link", { name: "Settings" }), "Open Settings from the left sidebar");
await t.click(t.page.getByRole("link", { name: "Notifications" }), "Open the Notifications tab");
await t.click(t.page.getByLabel("Email me"), "Turn on email alerts");
await t.show(t.page.locator("#msg"), "The page confirms it saved");
const video = await t.finish();
```

- `open` is the only direct load. Sign in before it with a stored session, or click through the login form on camera.
- `click(locator, caption)` moves the cursor, rings the target, shows the caption, then clicks. Scope the locator to the nav (`aside`, `nav`, `[role=tablist]`) so the viewer sees where the link lives.
- `type(locator, value, caption)` types key by key. `show(locator, caption)` holds on the result.
- Each caption says what the viewer does and where, in the viewer's words: "Open Reports from the left sidebar". Do not narrate code.
- `finish()` closes the recording, writes `steps.md` beside it and throws if the page changed without a click. A throw means the tutorial is not valid. Fix the script and rerun.
- A link that exists only in a menu stays hidden until opened. Click the menu first, then the link.
- Fill the app with believable data first. Never show secrets or real user data.

## Validate

A video passes only when a person can finish it without the script.

1. Pull frames: `ffmpeg -i <video> -vf "fps=1/1.6,scale=640:-1" <dir>/f%02d.png`, and a contact sheet with `-filter_complex tile=2x5` for a quick read.
2. Look at them. Every page change must follow a ringed click on a link that is on screen. The caption must match the frame. The final frame must show the feature's result.
3. Replay from the video alone. Spawn a fresh agent. Give it only the frames and the app's start URL, not the script, `steps.md` or the helper. It writes the steps it sees, then does them in a real browser using clicks only, no URL edits, and reports the first step where it got stuck.
4. If the replay gets stuck or has to guess, fix the script and record again. Ship only after a replay completes.

A browser cannot show a CLI, a library or a backend-only change. Do not fake a video for those. Post the command output as proof instead.
