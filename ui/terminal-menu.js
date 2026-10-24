function terminalMenu(tab) {
  const buffer = tab.term.buffer.active;
  const bottom = buffer.baseY + tab.term.rows;
  const lines = [];
  for (let i = buffer.baseY; i < bottom; i++) {
    const row = buffer.getLine(i);
    const text = row?.translateToString(true) ?? "";
    if (row?.isWrapped && lines.length) lines[lines.length - 1] += text;
    else lines.push(text);
  }
  return menuOn(lines);
}

/** The menu claude shows on a screen of `lines`, if one is up: the folder trust check, a permission
 * request, or any other choice. It is a column of options at the bottom of the screen, one marked
 * ❯, numbered or not, under the text it asks, which starts after the rule claude draws above. */
function menuOn(lines) {
  let end = lines.length - 1;
  while (end >= 0 && (!lines[end].trim() || /\b(Esc|Enter|Tab)\b/.test(lines[end])) && !CURSOR.test(lines[end])) end--;
  let cursor = end;
  while (cursor >= Math.max(0, end - 12) && !CURSOR.test(lines[cursor])) cursor--;
  if (cursor < Math.max(0, end - 12)) return null;
  const at = lines[cursor].search(CURSOR) + 2;
  const option = (l) => l.length > at && !l.slice(0, at).replace(CURSOR, "").trim() && l[at] !== " ";
  const continued = (l) => l.length > at && !l.slice(0, at + 1).trim();
  let first = cursor;
  while (first > 0 && (option(lines[first - 1]) || continued(lines[first - 1]))) first--;
  while (!option(lines[first])) first++;
  const options = [];
  for (let k = first; k <= end; k++) {
    if (option(lines[k])) options.push({ n: options.length + 1, label: lines[k].slice(at).trim().replace(/^\d+\.\s+/, ""), chosen: CURSOR.test(lines[k]) });
    else if (continued(lines[k]) && options.length) options.at(-1).label += ` ${lines[k].trim()}`;
    else return null;
  }
  if (options.length < 2) return null;
  const rule = lines.slice(0, first).findLastIndex((l) => /^\s*[─━]{8,}/.test(l));
  const above = lines.slice(rule + 1, first).map((l) => l.trim()).filter(Boolean);
  const dashed = (l) => /^[─━╌┄┈═-]+$/.test(l);
  const boxStart = above.findIndex(dashed);
  const boxEnd = above.findLastIndex(dashed);
  const boxed = (i) => boxStart >= 0 && i > boxStart && i < boxEnd;
  const box = above.filter((l, i) => boxed(i)).join("\n");
  const question = above.filter((l, i) => !dashed(l) && !boxed(i)).join("\n");
  return { question, box, options };
}
const CURSOR = /[❯›](?= )/;
