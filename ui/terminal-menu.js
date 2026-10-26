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

/** The dialog claude draws in place of its prompt for /usage, /status, /mcp and the like, if one is
 * up: the lines under the ▔ rule above it. These dialogs write nothing to the transcript. A ▔ rule
 * with claude's prompt box under it is only output, such as a file shown in the terminal. */
function dialogOn(lines) {
  const rule = lines.findLastIndex((l) => DIALOG_RULE.test(l));
  if (rule < 0) return null;
  const body = lines.slice(rule + 1);
  if (body.some((l) => /^\s*─{20,}\s*$/.test(l))) return null;
  while (body.length && !body.at(-1).trim()) body.pop();
  return body.length ? body : null;
}
const DIALOG_RULE = /▔{20,}/;

/** claude's dialog on `tab`'s screen as HTML in claude's own colors, or null when none is up. */
function terminalDialog(tab) {
  const buffer = tab.term.buffer.active;
  const rows = Array.from({ length: tab.term.rows }, (_, i) => buffer.getLine(buffer.baseY + i));
  const lines = rows.map((row) => row?.translateToString(true) ?? "");
  const body = dialogOn(lines);
  if (!body) return null;
  const top = lines.findLastIndex((l) => DIALOG_RULE.test(l)) + 1;
  const cell = buffer.getNullCell();
  // The 16 named colors as the terminal drawer's theme sets them, so the card reads like the drawer.
  const theme = tab.term.options.theme ?? {};
  const palette = ANSI.map((c, i) => theme[ANSI_KEYS[i]] ?? c);
  return rows.slice(top, top + body.length).map((row) => rowHtml(row, cell, palette)).join("\n");
}

function rowHtml(row, cell, palette) {
  let html = "";
  let run = { key: null, text: "" };
  const flush = () => {
    if (run.text) html += run.key ? `<span ${run.key}>${esc(run.text)}</span>` : esc(run.text);
  };
  for (let x = 0; x < row.length; x++) {
    row.getCell(x, cell);
    if (!cell.getWidth()) continue;
    let fg = cell.isFgRGB() ? hex(cell.getFgColor()) : cell.isFgPalette() ? paletteColor(cell.getFgColor(), palette) : null;
    let bg = cell.isBgRGB() ? hex(cell.getBgColor()) : cell.isBgPalette() ? paletteColor(cell.getBgColor(), palette) : null;
    const inverse = !!cell.isInverse();
    if (inverse) [fg, bg] = [bg, fg];
    const classes = [inverse && "inverse", cell.isBold() && "bold", cell.isDim() && "dim", cell.isItalic() && "italic", cell.isUnderline() && "underline"].filter(Boolean);
    const style = [fg && `color:${fg}`, bg && `background:${bg}`].filter(Boolean).join(";");
    const key = [classes.length && `class="${classes.join(" ")}"`, style && `style="${style}"`].filter(Boolean).join(" ") || null;
    if (key !== run.key) {
      flush();
      run = { key, text: "" };
    }
    run.text += cell.getChars() || " ";
  }
  if (!run.key) run.text = run.text.trimEnd();
  flush();
  return html;
}
const hex = (rgb) => `#${rgb.toString(16).padStart(6, "0")}`;
// xterm's own palette, for a terminal whose theme names no colors, as the terminal view's.
const ANSI = ["#2e3436", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf", "#555753", "#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec"];
const ANSI_NAMED = ["Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White"];
const ANSI_KEYS = [...ANSI_NAMED.map((c) => c.toLowerCase()), ...ANSI_NAMED.map((c) => `bright${c}`)];
function paletteColor(n, palette) {
  if (n < 16) return palette[n];
  if (n >= 232) return `rgb(${Array(3).fill(8 + (n - 232) * 10)})`;
  const steps = [0, 95, 135, 175, 215, 255];
  return `rgb(${steps[Math.floor((n - 16) / 36)]},${steps[Math.floor((n - 16) / 6) % 6]},${steps[(n - 16) % 6]})`;
}
