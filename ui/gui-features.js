/** Funcionalidades opcionais: interruptores gerais que o usuário liga e desliga na página General. */
const guiFeatures = { alwaysOnTop: true };
try {
  Object.assign(guiFeatures, JSON.parse(load("gui-features")));
} catch {}
const saveFeatures = () => save("gui-features", JSON.stringify(guiFeatures));

/** Aplica o always on top na janela atual, e mostra o app no Dock quando ele sai de cima das
 * outras janelas, para a janela nunca se perder. Sem efeito no navegador, que não tem janela nativa. */
async function applyAlwaysOnTop() {
  if (!TAURI?.window) return;
  const on = !!guiFeatures.alwaysOnTop;
  try {
    await TAURI.window.getCurrentWindow().setAlwaysOnTop(on);
  } catch (e) {
    console.warn("always on top", e);
    return;
  }
  try {
    await invoke("set_dock_shown", { shown: !on });
  } catch (e) {
    console.warn("dock", e);
  }
}
applyAlwaysOnTop();

/** A seção de funcionalidades opcionais da página General. Cada funcionalidade nova ganha uma linha aqui. */
async function featuresSettingsHtml() {
  // Só o app tem janela nativa e caffeinate; no navegador não há opcional a mostrar por enquanto.
  if (!TAURI?.window) return "";
  const [awake, background] = await Promise.all([invoke("get_keep_awake"), invoke("get_background")]);
  const row = (key, title, about, control, extra = "") => `<div class="side-setting" data-setting="${key}"><div><label for="feature-${key}">${title}</label><p class="meta">${about}</p>${extra}</div>${control}</div>`;
  const toggle = (key, on, title, about, extra = "") => row(key, title, about, `<label class="switch"><input type="checkbox" id="feature-${key}" aria-label="${title}"${on ? " checked" : ""}><span></span></label>`, extra);
  return `<section id="features-settings"><h2 class="looks-title">Optional features</h2>
    ${toggle("keepAwake", awake, `Keep this ${COMPUTER} awake`, `While this is on, the ${COMPUTER} and its display don't go to sleep, so sessions keep working when you step away. It turns off when you switch it off or quit pinned.`, `<p class="error" id="awake-error"></p>`)}
    ${toggle("alwaysOnTop", guiFeatures.alwaysOnTop, "Always on top", `Keep the pinned window above other apps, even in their fullscreen spaces. When off, pinned shows in ${IS_MAC ? "the Dock" : "the taskbar"} so you can bring it back.`)}
    ${machineBackgroundHtml(LOCAL, { background })}</section>`;
}
function bindFeaturesSettings() {
  const awake = document.getElementById("feature-keepAwake");
  if (awake) {
    awake.onchange = async () => {
      try {
        await invoke("set_keep_awake", { on: awake.checked });
        document.getElementById("awake-error").textContent = "";
      } catch (err) {
        awake.checked = !awake.checked;
        document.getElementById("awake-error").textContent = err;
      }
    };
  }
  const box = document.getElementById("feature-alwaysOnTop");
  if (!box) return;
  box.onchange = async () => {
    const on = box.checked;
    try {
      await TAURI.window.getCurrentWindow().setAlwaysOnTop(on);
    } catch (e) {
      box.checked = !on;
      console.warn("always on top", e);
      return;
    }
    guiFeatures.alwaysOnTop = on;
    saveFeatures();
    try {
      await invoke("set_dock_shown", { shown: !on });
    } catch (e) {
      console.warn("dock", e);
    }
  };
}
