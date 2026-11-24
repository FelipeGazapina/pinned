/** The desktop app's Memory page: what the app, its webview helpers, each thread's and terminal's
 * processes and pinned's other children hold. The `memory_report` command reads it when the page
 * opens and when Refresh is pressed, never in the background. Copy gives the text for an issue. */

/** The last report the page showed, which Copy writes out. */
let memoryShown = null;

/** Bytes in MB, or in GB with one decimal from 1024 MB up. */
function memorySize(bytes) {
  const mb = bytes / (1024 * 1024);
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
}
/** A group's or row's size, or `could not read` when none of its processes could be read. */
const memoryAmount = (sum) => (sum.processes > 0 && sum.unreadable === sum.processes ? "could not read" : memorySize(sum.bytes));
function memoryCount(sum) {
  const count = `${sum.processes} ${sum.processes === 1 ? "process" : "processes"}`;
  return sum.unreadable ? `${count}, ${sum.unreadable} could not be read` : count;
}
function memoryUptime(secs) {
  const [d, h, m] = [Math.floor(secs / 86400), Math.floor((secs % 86400) / 3600), Math.floor((secs % 3600) / 60)];
  return d ? `${d} d ${h} h` : h ? `${h} h ${m} min` : `${m} min`;
}
const memoryRows = (group) => [...(group.rows ?? [])].sort((a, b) => b.bytes - a.bytes);
const memoryRowWhat = (row) => [row.agent, row.state].filter(Boolean).join(", ");

/** The report as plain text to paste into a GitHub issue. */
function memoryReportText(report) {
  const lines = [
    `pinned ${report.version} on ${report.os}, ${report.measure}`,
    `Uptime: ${memoryUptime(report.uptime_secs)}`,
    `Total: ${memoryAmount(report)}, ${memoryCount(report)}`,
  ];
  for (const group of report.groups) {
    lines.push(`${group.name}: ${memoryAmount(group)}, ${memoryCount(group)}`);
    for (const row of memoryRows(group)) lines.push(`  ${row.label} (${memoryRowWhat(row)}): ${memoryAmount(row)}, ${memoryCount(row)}`);
  }
  return `${lines.join("\n")}\n`;
}

function memoryLine(label, meta, sum, cls = "") {
  return `<div class="side-setting${cls}"><div><label>${esc(label)}</label><p class="meta">${esc(meta)}</p></div><span class="memory-size">${esc(memoryAmount(sum))}</span></div>`;
}

function memoryHtml(report, at) {
  const groups = report.groups.map((group) => memoryLine(group.name, memoryCount(group), group) + memoryRows(group).map((row) => memoryLine(row.label, [memoryRowWhat(row), memoryCount(row)].join(" · "), row, " memory-row")).join(""));
  return `<div class="side-setting"><div><label>Read at ${esc(at.toLocaleTimeString())}</label><p class="meta">Up ${esc(memoryUptime(report.uptime_secs))} · ${esc(report.measure)}</p></div>
    <span class="memory-actions"><button type="button" class="outline" data-memory="refresh">${icon("rotate-cw")}Refresh</button> <button type="button" class="outline" data-memory="copy">${icon("copy")}<span>Copy</span></button></span></div>
    ${memoryLine("Total", memoryCount(report), report)}${groups.join("")}`;
}

async function readMemory() {
  let html;
  try {
    memoryShown = await invoke("memory_report");
    html = memoryHtml(memoryShown, new Date());
  } catch (error) {
    memoryShown = null;
    html = `<div class="card"><p>${esc(String(error?.message ?? error))}</p></div>`;
  }
  if (page !== "memory" || !settingsOpen()) return;
  settingsPage.innerHTML = `<h2>Memory</h2><section id="memory">${html}</section>`;
  document.getElementById("memory").onclick = async (e) => {
    const button = e.target.closest("[data-memory]");
    if (button?.dataset.memory === "refresh") {
      button.disabled = true;
      return readMemory();
    }
    if (button?.dataset.memory !== "copy" || !memoryShown) return;
    try {
      await navigator.clipboard.writeText(memoryReportText(memoryShown));
      button.querySelector("span").textContent = "Copied";
    } catch (error) {
      console.warn("copy memory report", error);
    }
  };
}

function showMemory() {
  settingsPage.innerHTML = `<h2>Memory</h2><section id="memory"><p class="meta">Reading memory…</p></section>`;
  return readMemory();
}

if (TAURI) settingsSection({ page: "memory", label: "Memory", icon: "chart-column", show: () => showMemory(), search: [{ title: "Memory", terms: ["ram memory usage leak processes footprint resident webview helpers threads terminals report copy"] }] });
