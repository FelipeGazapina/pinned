function screenLines(tab) {
  const buffer = tab.term.buffer.active;
  const bottom = buffer.baseY + tab.term.rows;
  const lines = [];
  for (let i = buffer.baseY; i < bottom; i++) {
    const row = buffer.getLine(i);
    const text = row?.translateToString(true) ?? "";
    if (row?.isWrapped && lines.length) lines[lines.length - 1] += text;
    else lines.push(text);
  }
  return lines;
}
const terminalMenu = (tab) => menuOn(screenLines(tab));
const terminalMode = (tab) => modeOn(screenLines(tab));

/** The menu claude shows on a screen of `lines`, if one is up: the folder trust check, a permission
 * request, or any other choice. It is a column of options at the bottom of the screen, one marked
 * ❯, numbered or not, under the text it asks, which starts after the rule claude draws above. A
 * question's numbered options may have a rule above the last few. Each option's `label` is all of
 * its lines; `head` is its first and `more` the rest, which a question uses for a description. */
function menuOn(lines) {
  let end = lines.length - 1;
  while (end >= 0 && (!lines[end].trim() || /\b(Esc|Enter|Tab|ctrl\+\w)\b/.test(lines[end])) && !CURSOR.test(lines[end])) end--;
  let cursor = end;
  while (cursor >= Math.max(0, end - 16) && !CURSOR.test(lines[cursor])) cursor--;
  if (cursor < Math.max(0, end - 16)) return null;
  const at = lines[cursor].search(CURSOR) + 2;
  const option = (l) => l.length > at && !l.slice(0, at).replace(CURSOR, "").trim() && l[at] !== " ";
  const continued = (l) => l.length > at && !l.slice(0, at + 1).trim();
  const numbered = /^\d+\.\s/.test(lines[cursor].slice(at));
  const rule = (l) => numbered && /^\s*[─━]{8,}\s*$/.test(l);
  let first = cursor;
  while (first > 0 && (option(lines[first - 1]) || continued(lines[first - 1]) || (rule(lines[first - 1]) && first - 1 > 0 && option(lines[first - 2])))) first--;
  while (!option(lines[first])) first++;
  const options = [];
  for (let k = first; k <= end; k++) {
    const text = lines[k].slice(at).trim();
    if (option(lines[k])) {
      const number = text.match(/^(\d+)\.\s+/);
      if (numbered && Number(number?.[1]) !== options.length + 1) return null;
      const head = text.slice(number?.[0].length ?? 0);
      options.push({ n: options.length + 1, label: head, head, more: "", chosen: CURSOR.test(lines[k]) });
    } else if (continued(lines[k]) && options.length) {
      options.at(-1).label += ` ${lines[k].trim()}`;
      options.at(-1).more = `${options.at(-1).more} ${lines[k].trim()}`.trim();
    } else if (!rule(lines[k])) return null;
  }
  if (options.length < 2) return null;
  const top = lines.slice(0, first).findLastIndex((l) => /^\s*[─━]{8,}/.test(l));
  const above = lines.slice(top + 1, first).map((l) => l.trim()).filter(Boolean);
  const dashed = (l) => /^[─━╌┄┈═-]+$/.test(l);
  const boxStart = above.findIndex(dashed);
  const boxEnd = above.findLastIndex(dashed);
  const boxed = (i) => boxStart >= 0 && i > boxStart && i < boxEnd;
  const box = above.filter((l, i) => boxed(i)).join("\n");
  const question = above.filter((l, i) => !dashed(l) && !boxed(i)).join("\n");
  const hints = lines.slice(end + 1).join(" ");
  return { question, box, options, amend: /Tab to amend/.test(hints) };
}
const CURSOR = /[❯›](?= )/;

/** Claude's permission mode, from the line it draws under its prompt box, such as
 * "⏵⏵ accept edits on (shift+tab to cycle)"; a claude older than the manual mode's own line shows
 * only "? for shortcuts" in it. */
function modeOn(lines) {
  const bottom = lines.filter((l) => l.trim()).slice(-4).join("\n");
  const named = [...bottom.matchAll(/(?:⏵⏵|⏸)\s*(auto mode|manual mode|accept edits|plan mode|bypass permissions) on/g)].at(-1);
  if (named) return { "auto mode": "auto", "manual mode": "manual", "accept edits": "accept", "plan mode": "plan", "bypass permissions": "bypass" }[named[1]];
  return /\? for shortcuts/.test(bottom) ? "manual" : null;
}
