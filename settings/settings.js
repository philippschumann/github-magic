// The settings page: the extension's options page and the app's Settings window. It stores nothing itself: every
// change is one `set` request to the app, which checks and saves it.
const $ = (id) => document.getElementById(id);
// Setting (its <select> id) -> the list of choices the app detects.
const PICKS = { ide: "ides", iosIde: "iosIdes", emulator: "avds", simulator: "simulators" };
// What a repo of each platform needs for device runs.
const APP = { android: { key: "appId", label: "App ID", hint: "com.example.myapp" }, ios: { key: "scheme", label: "Scheme", hint: "MyApp" } };

function show(text, isError = false) {
  $("status").textContent = text;
  $("status").classList.toggle("error", isError);
}

// Saves one setting. A refused value is replaced by what is stored.
async function set(key, value, repo) {
  const res = await call({ action: "set", key, value, ...(repo && { repo }) });
  show(res.ok ? "Saved" : res.error ?? "No response from Github Magic", !res.ok);
  if (!res.ok) load();
}

function repoRow(name, repo, platform) {
  const { key, label, hint } = APP[platform];
  const row = document.createElement("li");
  const title = document.createElement("strong");
  title.textContent = name;
  const path = document.createElement("div");
  path.className = "path";
  path.textContent = `${platform === "ios" ? "iOS" : "Android"} · ${repo.path}`;
  const app = Object.assign(document.createElement("input"), { type: "text", value: repo[key] ?? "", placeholder: hint, spellcheck: false });
  app.id = `${key}-${name}`;
  app.addEventListener("change", () => set(key, app.value.trim(), name));
  const appLabel = Object.assign(document.createElement("label"), { htmlFor: app.id, textContent: label });
  const remove = Object.assign(document.createElement("button"), { type: "button", textContent: "Remove" });
  remove.setAttribute("aria-label", `Remove ${name}`);
  remove.addEventListener("click", async () => {
    const res = await call({ action: "remove_repo", repo: name });
    show(res.ok ? `Removed ${name}` : res.error, !res.ok);
    load();
  });
  const fields = document.createElement("div");
  fields.className = "row";
  fields.append(appLabel, app, remove);
  row.append(title, path, fields);
  return row;
}

// Shows what the app has stored. The field being edited is left alone.
function render({ settings, platforms, ...choices }) {
  const editing = document.activeElement;
  if (!$("repos").contains(editing)) {
    $("repos").replaceChildren(...Object.entries(settings.repos).map(([name, repo]) => repoRow(name, repo, platforms[name])));
  }
  for (const [key, list] of Object.entries(PICKS)) {
    if ($(key) === editing) continue;
    const options = choices[list].map((choice) => new Option(choice.label ?? choice, choice.id ?? choice));
    $(key).replaceChildren(new Option("Not chosen", ""), ...options);
    $(key).value = settings[key]; // one that is gone (uninstalled, deleted) shows as not chosen
    if ($(key).selectedIndex < 0) $(key).value = "";
  }
  if ($("worktreeDir") !== editing) $("worktreeDir").value = settings.worktreeDir;
  if ($("phoneModels") !== editing) $("phoneModels").value = settings.phoneModels.join("\n");
  $("openAtLogin").checked = settings.openAtLogin;
}

async function load() {
  const res = await call({ action: "settings" });
  $("missing").hidden = !res.missing;
  $("fields").disabled = !res.ok;
  if (res.ok) render(res);
  else if (!res.missing) show(res.error ?? "No response from Github Magic", true);
}

$("worktreeDir").addEventListener("change", () => set("worktreeDir", $("worktreeDir").value.trim()));
for (const key of Object.keys(PICKS)) $(key).addEventListener("change", () => set(key, $(key).value));
$("phoneModels").addEventListener("change", () => {
  set("phoneModels", $("phoneModels").value.split("\n").map((line) => line.trim()).filter(Boolean));
});
$("openAtLogin").addEventListener("change", () => set("openAtLogin", $("openAtLogin").checked));

// A browser's own pickers never reveal an absolute path, so the app shows the system's folder dialog.
$("browse").addEventListener("click", async () => {
  $("browse").disabled = true;
  const res = await call({ action: "choose_folder", current: $("worktreeDir").value.trim() });
  $("browse").disabled = false;
  if (res.cancelled) return;
  if (!res.ok) return show(res.error ?? "No response from Github Magic", true);
  $("worktreeDir").value = res.path;
  set("worktreeDir", res.path);
});

$("add").addEventListener("click", async () => {
  $("add").disabled = true;
  const res = await call({ action: "add_repo" });
  $("add").disabled = false;
  if (res.cancelled) return;
  show(res.ok ? `Added ${res.repo}` : res.error ?? "No response from Github Magic", !res.ok);
  load();
});

onChange(load);
load();
