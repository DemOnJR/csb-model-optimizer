"use strict";
// UI of the model optimizer. With Tauri it calls the app's commands; opened as a plain page with
// ?mock it uses mock-report.json, so the layout can be checked in a browser.

const T = window.__TAURI__;
const MOCK = !T && location.search.includes("mock");
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

async function call(cmd, args) {
  if (T) return T.core.invoke(cmd, args);
  if (cmd === "default_output") return args.input.replace(/[\\/]+$/, "") + "_optimized";
  if (cmd === "folder_state") return { exists: true, empty: cmd === "x", game_root: true };
  if (cmd === "pick_folder") return "D:\\Servers\\zp\\cstrike";
  if (cmd === "optimize") {
    const stages = ["reading files", "repacking models", "grouping shared animation", "writing files"];
    for (let i = 0; i < 40; i++) {
      onProgress({ stage: stages[Math.floor(i / 10)], done: i % 10, total: 10 });
      await new Promise((r) => setTimeout(r, 25));
    }
    return (await fetch("mock-report.json")).json();
  }
  return null;
}

const MB = 1048576;
function size(b) {
  if (b >= MB * 1024) return (b / MB / 1024).toFixed(2) + " GB";
  if (b >= MB) return (b / MB).toFixed(1) + " MB";
  if (b >= 1024) return (b / 1024).toFixed(0) + " KB";
  return b + " B";
}
const pct = (a, b) => (b ? Math.round((100 * a) / b) : 100) + "%";
function dur(sec) {
  if (sec < 1) return "<1 s";
  if (sec < 90) return Math.round(sec) + " s";
  if (sec < 5400) return Math.round(sec / 60) + " min";
  return (sec / 3600).toFixed(1) + " h";
}

let report = null;

// ---- form ----
async function noteFor(which) {
  const v = $(which).value.trim();
  const note = $(which + "Note");
  note.textContent = "";
  if (!v) return;
  const st = await call("folder_state", { path: v });
  if (which === "input") {
    if (!st.exists) note.textContent = "Folder not found.";
    else if (!st.game_root) note.textContent = "No models/, maps/, sound/ or sprites/ inside: pick the cstrike (or cstrike_downloads) folder itself.";
  } else if (st.exists && !st.empty) {
    note.textContent = "Not empty: files with the same names are overwritten.";
  }
}

document.querySelectorAll("[data-pick]").forEach((b) =>
  b.addEventListener("click", async () => {
    const which = b.dataset.pick;
    const p = await call("pick_folder", { start: $(which).value || $("input").value || null });
    if (!p) return;
    $(which).value = p;
    if (which === "input" && !$("output").dataset.touched) {
      $("output").value = await call("default_output", { input: p });
      noteFor("output");
    }
    noteFor(which);
  })
);
$("input").addEventListener("change", async () => {
  if (!$("output").dataset.touched && $("input").value.trim()) $("output").value = await call("default_output", { input: $("input").value.trim() });
  noteFor("input");
  noteFor("output");
});
$("output").addEventListener("input", () => ($("output").dataset.touched = "1"));
$("output").addEventListener("change", () => noteFor("output"));

function onProgress(p) {
  $("progress").hidden = false;
  $("progressBar").style.width = (p.total ? (100 * Math.min(p.done + 1, p.total)) / p.total : 100) + "%";
  $("progressText").textContent = p.stage + (p.total > 1 ? ` (${Math.min(p.done + 1, p.total)} of ${p.total})` : "");
}
if (T) T.event.listen("progress", (e) => onProgress(e.payload));

$("form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const input = $("input").value.trim();
  const output = $("output").value.trim();
  $("error").hidden = true;
  if (!input || !output) {
    showError("Pick the game folder and an output folder first.");
    return;
  }
  const options = { repack: $("repack").checked, share: $("share").checked, gzip: $("gzip").value, models_only: $("modelsOnly").checked };
  $("run").disabled = true;
  $("run").textContent = "Working…";
  try {
    report = await call("optimize", { input, output, options });
    show();
  } catch (err) {
    showError(String(err));
  } finally {
    $("run").disabled = false;
    $("run").textContent = "Optimize";
    $("progress").hidden = true;
  }
});

function showError(msg) {
  $("error").textContent = msg;
  $("error").hidden = false;
}

// ---- tabs ----
document.querySelectorAll("#tabs button").forEach((b) =>
  b.addEventListener("click", () => {
    document.querySelectorAll("#tabs button").forEach((x) => x.classList.toggle("on", x === b));
    document.querySelectorAll(".tab").forEach((s) => (s.hidden = s.id !== b.dataset.tab));
  })
);

// ---- report ----
function show() {
  const t = report.totals;
  const gz = report.options.gzip !== "off";
  const best = gz ? t.after_gz : t.after;
  $("empty").hidden = true;
  $("tabs").hidden = false;
  document.querySelector('#tabs button[data-tab="overview"]').click();
  const n = (a) => report.files.filter((f) => f.action === a).length;
  $("summary").hidden = false;
  $("summary").innerHTML = `
    <div class="mute">What a client downloads</div>
    <div class="big"><b>${size(t.before)}</b><span class="arrow">→</span><b>${size(best)}</b><span class="save">−${pct(t.before - best, t.before)}</span></div>
    <div class="kv">
      <span>Files</span><span>${t.files}</span>
      <span>Models packed</span><span>${n("repacked")}</span>
      <span>Models sharing animation</span><span>${n("slim")}</span>
      <span>Shared animation files</span><span>${n("shared")}</span>
      <span>Left as they were</span><span>${n("skipped")}</span>
      <span>Time</span><span>${report.seconds.toFixed(1)} s</span>
    </div>
    <div class="actions">
      <button type="button" id="openOut">Open output folder</button>
      <button type="button" id="saveRep">Save report</button>
    </div>`;
  $("openOut").onclick = () => call("open_folder", { path: report.output }).catch(showError);
  $("saveRep").onclick = () =>
    call("save_report", { path: report.output.replace(/[\\/]+$/, "") + ".report.json", report })
      .then(() => ($("saveRep").textContent = "Saved next to the output folder"))
      .catch(showError);
  renderOverview();
  renderFiles();
  renderShared();
  renderWarnings();
}

function bars(t, max, gz) {
  const row = (label, v, color) =>
    `<div class="rowbar"><div class="track"><i style="width:${(100 * v) / max}%;background:${color}"></i></div><span>${label} ${size(v)}</span></div>`;
  return `<div class="rows">
    ${row("before", t.before, "var(--before)")}
    ${gz ? row("gz only", t.before_gz, "color-mix(in srgb,var(--gz) 45%,var(--before))") : ""}
    ${row("optimized", t.after, "var(--after)")}
    ${gz ? row("optimized + gz", t.after_gz, "var(--gz)") : ""}
  </div>`;
}

function renderOverview() {
  const t = report.totals;
  const gz = report.options.gzip !== "off";
  const best = gz ? t.after_gz : t.after;
  const speeds = [
    ["Fast download at 10 MB/s", 10 * MB],
    ["Fast download at 2 MB/s", 2 * MB],
    ["In-game download at 50 KB/s (no gz)", 50 * 1024],
  ];
  const max = Math.max(...report.categories.map((c) => c[1].before), 1);
  $("overview").innerHTML = `
    <div class="tiles">
      ${speeds
        .map(([label, bps], i) => {
          const after = i === 2 ? t.after : best;
          return `<div class="tile"><div class="t">${label}</div><div class="v">${dur(t.before / bps)} → ${dur(after / bps)}</div><div class="d mute">${size(t.before)} → ${size(after)}</div></div>`;
        })
        .join("")}
    </div>
    <h3>Everything</h3>
    <div class="legend">
      <span><i style="background:var(--before)"></i>before</span>
      ${gz ? `<span><i style="background:color-mix(in srgb,var(--gz) 45%,var(--before))"></i>only gzip</span>` : ""}
      <span><i style="background:var(--after)"></i>optimized models</span>
      ${gz ? `<span><i style="background:var(--gz)"></i>optimized models + gzip (what clients get)</span>` : ""}
    </div>
    <div class="cmp"><div class="name">All files<small>${t.files} files</small></div>${bars(t, t.before, gz)}</div>
    <h3>By kind</h3>
    <div class="cmp">
      ${report.categories.map(([name, c]) => `<div class="name">${esc(name)}<small>${c.files} files</small></div>${bars(c, max, gz)}`).join("")}
    </div>`;
}

function renderFiles() {
  const q = $("filter").value.trim().toLowerCase();
  const changed = $("changedOnly").checked;
  const gz = report.options.gzip !== "off";
  const key = $("sort").value;
  let list = report.files.filter((f) => (!changed || f.action !== "copied") && (!q || f.path.toLowerCase().includes(q)));
  const saved = (f) => f.before - (gz ? f.after_gz : f.after);
  list.sort(key === "path" ? (a, b) => a.path.localeCompare(b.path) : key === "before" ? (a, b) => b.before - a.before : (a, b) => saved(b) - saved(a));
  const shown = list.slice(0, 600);
  const max = Math.max(...shown.map((f) => Math.max(f.before, f.after)), 1);
  $("fileCount").textContent = `${list.length} files${list.length > shown.length ? `, first ${shown.length} shown` : ""}`;
  $("fileList").innerHTML = shown
    .map((f) => {
      const after = gz ? f.after_gz : f.after;
      const bar = (v, color, label) =>
        `<div class="rowbar"><div class="track"><i style="width:${(100 * v) / max}%;background:${color}"></i></div><span>${label} ${size(v)}</span></div>`;
      return `<div class="file">
        <div class="p" title="${esc(f.path)}">${esc(f.path)}</div>
        <div class="meta"><span class="tag ${f.action}">${f.action}</span><span title="${esc(f.note)}">${esc(f.note)}</span></div>
        <div class="nums">
          ${f.action === "shared" ? "" : bar(f.before, "var(--before)", "before")}
          ${bar(after, gz ? "var(--gz)" : "var(--after)", gz ? "after + gz" : "after")}
        </div>
      </div>`;
    })
    .join("");
}
["filter", "changedOnly", "sort"].forEach((id) => $(id).addEventListener("input", () => report && renderFiles()));

function renderShared() {
  const s = report.shared;
  if (!s.length) {
    $("shared").innerHTML = `<p class="mute">No animation was shared${report.options.share ? ": no player models here have enough animation in common (or sharing would make a model use more memory than today)." : " (turned off)."}</p>`;
    return;
  }
  const res = s.map((x) => x.name).join("\n");
  const amxx = s.map((x) => `precache_generic("${x.name}")`).join("\n");
  $("shared").innerHTML = `
    <p>These files hold the animation of the models listed under each. <b>Every client that downloads one of those models also needs its file</b>, so the server must send them: add the lines below to <code>maps/&lt;map&gt;.res</code>, or call <code>precache_generic</code> next to the skins' <code>precache_model</code>. The same list is in <code>shared_anims.txt</code> in the output folder.</p>
    <div class="actions"><button type="button" id="copyRes">Copy .res lines</button><button type="button" id="copyAmxx">Copy precache_generic lines</button></div>
    <pre>${esc(res)}</pre>
    ${s
      .map(
        (x) => `<div class="card"><div class="p">${esc(x.name)} <span class="mute">· ${size(x.size)}</span></div>
          <ul>${x.users.map((u) => `<li>${esc(u)}</li>`).join("")}</ul></div>`
      )
      .join("")}`;
  const copy = (btn, text) => navigator.clipboard.writeText(text).then(() => (btn.textContent = "Copied"));
  $("copyRes").onclick = (e) => copy(e.target, res + "\n");
  $("copyAmxx").onclick = (e) => copy(e.target, amxx + "\n");
}

function renderWarnings() {
  const p = report.problems.map((m) => `<div class="card err"><div>${esc(m)}</div></div>`).join("");
  const order = { warn: 0, info: 1 };
  const lints = [...report.lints].sort((a, b) => order[a.level] - order[b.level] || b.wasted - a.wasted);
  const l = lints
    .map(
      (x) => `<div class="card ${x.level}"><div class="p">${esc(x.path)}</div><div>${esc(x.message)}${x.wasted ? ` <span class="mute">· ${size(x.wasted)} to save</span>` : ""}</div></div>`
    )
    .join("");
  const warnN = report.problems.length + lints.filter((x) => x.level === "warn").length;
  $("warnCount").textContent = warnN || "";
  $("warnings").innerHTML =
    (p ? `<h3>Kept as they were (a check failed)</h3>${p}` : "") +
    (l ? `<h3>Found in the files</h3><p class="mute">Not fixed automatically: these need a decision (delete a file, recompile a model).</p>${l}` : "") +
    (!p && !l ? `<p class="mute">Nothing to report.</p>` : "");
}

if (MOCK) {
  $("input").value = "D:\\Servers\\zp\\cstrike";
  $("output").value = "D:\\Servers\\zp\\cstrike_optimized";
}
